use std::convert::Infallible;

use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, Sse},
    },
};
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    application::state::SharedState,
    protocol::ERROR_INVALID_REQUEST,
    rpc::{SessionContext, methods, policy},
    security::auth,
    storage::now_unix_ms,
};

use super::compat::{authorize_gateway_http, extract_text_content, normalize_segment};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatCompletionsRequest {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    messages: Vec<ChatMessage>,
    #[serde(default)]
    user: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatMessage {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<Value>,
    #[serde(default)]
    name: Option<String>,
}

pub async fn chat_completions_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Err(reason) = authorize_gateway_http(&state, &headers) {
        let message = auth::auth_failure_error(reason).message;
        return openai_error(StatusCode::UNAUTHORIZED, &message, "authentication_error");
    }

    let Json(raw_payload) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return openai_error(
                StatusCode::BAD_REQUEST,
                "Invalid JSON body.",
                "invalid_request_error",
            );
        }
    };

    let payload: ChatCompletionsRequest = match serde_json::from_value(raw_payload) {
        Ok(payload) => payload,
        Err(_) => {
            return openai_error(
                StatusCode::BAD_REQUEST,
                "Invalid request body.",
                "invalid_request_error",
            );
        }
    };

    let model = payload
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("reclaw-core")
        .to_owned();

    let Some(prompt) = build_prompt(&payload.messages) else {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "Missing user message in `messages`.",
            "invalid_request_error",
        );
    };

    let session_key = resolve_openai_session_key(&model, payload.user.as_deref());
    let completion_id = format!("chatcmpl_{}", uuid::Uuid::new_v4());
    let params = json!({
        "sessionKey": session_key,
        "message": prompt,
        "idempotencyKey": format!("openai-{completion_id}"),
    });
    let session = SessionContext {
        conn_id: format!("http-openai-{}", uuid::Uuid::new_v4()),
        role: "operator".to_owned(),
        scopes: policy::default_operator_scopes(),
        client_id: "openai-http".to_owned(),
        client_mode: "openai-http".to_owned(),
    };

    let rpc_result = methods::chat::handle_send(&state, &session, Some(&params)).await;
    let rpc_payload = match rpc_result {
        Ok(payload) => payload,
        Err(error) => {
            let status = if error.code == ERROR_INVALID_REQUEST {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            let error_type = if status == StatusCode::BAD_REQUEST {
                "invalid_request_error"
            } else {
                "api_error"
            };
            return openai_error(status, &error.message, error_type);
        }
    };

    let assistant_text = rpc_payload
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("No response from Reclaw Core.");
    let created = now_unix_ms().checked_div(1_000).unwrap_or(0);

    if payload.stream {
        return stream_completion_response(&completion_id, &model, created, assistant_text);
    }

    (
        StatusCode::OK,
        Json(json!({
            "id": completion_id,
            "object": "chat.completion",
            "created": created,
            "model": model,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": assistant_text,
                },
                "finish_reason": "stop",
            }],
            "usage": {
                "prompt_tokens": 0,
                "completion_tokens": 0,
                "total_tokens": 0,
            }
        })),
    )
        .into_response()
}

fn openai_error(status: StatusCode, message: &str, error_type: &str) -> Response {
    (
        status,
        Json(json!({
            "error": {
                "message": message,
                "type": error_type,
            }
        })),
    )
        .into_response()
}

fn stream_completion_response(
    completion_id: &str,
    model: &str,
    created: u64,
    content: &str,
) -> Response {
    let role_chunk = json!({
        "id": completion_id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {
                "role": "assistant"
            }
        }]
    })
    .to_string();

    let content_chunk = json!({
        "id": completion_id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {
                "content": content
            },
            "finish_reason": "stop"
        }]
    })
    .to_string();

    let events = vec![role_chunk, content_chunk, "[DONE]".to_owned()];
    let stream = stream::iter(
        events
            .into_iter()
            .map(|line| Ok::<Event, Infallible>(Event::default().data(line))),
    );

    Sse::new(stream).into_response()
}

fn build_prompt(messages: &[ChatMessage]) -> Option<String> {
    let mut system_parts = Vec::new();
    let mut conversation = Vec::new();
    let mut has_user_message = false;

    for message in messages {
        let role = message
            .role
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        if role.is_empty() {
            continue;
        }

        let content = message
            .content
            .as_ref()
            .map(extract_text_content)
            .unwrap_or_default();
        let content = content.trim();
        if content.is_empty() {
            continue;
        }

        match role.as_str() {
            "system" | "developer" => system_parts.push(content.to_owned()),
            "assistant" => conversation.push(format!("Assistant: {content}")),
            "user" => {
                has_user_message = true;
                conversation.push(format!("User: {content}"));
            }
            "tool" | "function" => {
                let name = message
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Tool");
                conversation.push(format!("{name}: {content}"));
            }
            _ => {}
        }
    }

    if !has_user_message {
        return None;
    }

    let mut sections = Vec::new();
    if !system_parts.is_empty() {
        sections.push(format!("System:\n{}", system_parts.join("\n\n")));
    }
    if !conversation.is_empty() {
        sections.push(conversation.join("\n"));
    }

    Some(sections.join("\n\n"))
}

fn resolve_openai_session_key(model: &str, user: Option<&str>) -> String {
    let agent_id = normalize_segment(model);
    let conversation = user
        .map(normalize_segment)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "default".to_owned());

    format!(
        "agent:{}:openai:chat:{conversation}",
        if agent_id.is_empty() {
            "main"
        } else {
            &agent_id
        }
    )
}

#[cfg(test)]
mod tests {
    use super::build_prompt;

    #[test]
    fn build_prompt_requires_user_message() {
        let messages: Vec<super::ChatMessage> = Vec::new();
        let prompt = build_prompt(&messages);
        assert!(prompt.is_none());
    }
}
