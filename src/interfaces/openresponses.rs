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
struct CreateResponseRequest {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    input: Option<Value>,
    #[serde(default)]
    user: Option<String>,
}

pub async fn responses_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Err(reason) = authorize_gateway_http(&state, &headers) {
        let message = auth::auth_failure_error(reason).message;
        return responses_error(StatusCode::UNAUTHORIZED, &message, "authentication_error");
    }

    let Json(raw_payload) = match payload {
        Ok(payload) => payload,
        Err(_) => {
            return responses_error(
                StatusCode::BAD_REQUEST,
                "Invalid JSON body.",
                "invalid_request_error",
            );
        }
    };

    let payload: CreateResponseRequest = match serde_json::from_value(raw_payload) {
        Ok(payload) => payload,
        Err(_) => {
            return responses_error(
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

    let Some(prompt) = extract_input_prompt(payload.input.as_ref()) else {
        return responses_error(
            StatusCode::BAD_REQUEST,
            "Missing input text in `input`.",
            "invalid_request_error",
        );
    };

    let response_id = format!("resp_{}", uuid::Uuid::new_v4());
    let session_key = resolve_openresponses_session_key(&model, payload.user.as_deref());
    let params = json!({
        "sessionKey": session_key,
        "message": prompt,
        "idempotencyKey": format!("openresponses-{response_id}"),
    });
    let session = SessionContext {
        conn_id: format!("http-openresponses-{}", uuid::Uuid::new_v4()),
        role: "operator".to_owned(),
        scopes: policy::default_operator_scopes(),
        client_id: "openresponses-http".to_owned(),
        client_mode: "openresponses-http".to_owned(),
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
            return responses_error(status, &error.message, error_type);
        }
    };

    let assistant_text = rpc_payload
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("No response from Reclaw Core.");
    let created = now_unix_ms().checked_div(1_000).unwrap_or(0);

    if payload.stream {
        return stream_response(&response_id, &model, created, assistant_text);
    }

    (
        StatusCode::OK,
        Json(build_response_resource(
            &response_id,
            &model,
            created,
            "completed",
            Some(assistant_text),
        )),
    )
        .into_response()
}

fn responses_error(status: StatusCode, message: &str, error_type: &str) -> Response {
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

fn stream_response(response_id: &str, model: &str, created: u64, assistant_text: &str) -> Response {
    let created_event = Event::default().event("response.created").data(
        json!({
            "type": "response.created",
            "response": build_response_resource(response_id, model, created, "in_progress", None)
        })
        .to_string(),
    );

    let output_item_id = format!("msg_{}", uuid::Uuid::new_v4());
    let delta_event = Event::default().event("response.output_text.delta").data(
        json!({
            "type": "response.output_text.delta",
            "response_id": response_id,
            "item_id": output_item_id,
            "delta": assistant_text,
        })
        .to_string(),
    );

    let completed_event = Event::default().event("response.completed").data(
        json!({
            "type": "response.completed",
            "response": build_response_resource(response_id, model, created, "completed", Some(assistant_text))
        })
        .to_string(),
    );

    let done_event = Event::default().data("[DONE]");

    let stream = stream::iter(
        vec![created_event, delta_event, completed_event, done_event]
            .into_iter()
            .map(Ok::<Event, Infallible>),
    );

    Sse::new(stream).into_response()
}

fn build_response_resource(
    response_id: &str,
    model: &str,
    created: u64,
    status: &str,
    assistant_text: Option<&str>,
) -> Value {
    let output = if let Some(text) = assistant_text {
        vec![json!({
            "type": "message",
            "id": format!("msg_{}", uuid::Uuid::new_v4()),
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": text,
            }],
            "status": "completed"
        })]
    } else {
        Vec::new()
    };

    json!({
        "id": response_id,
        "object": "response",
        "created_at": created,
        "status": status,
        "model": model,
        "output": output,
        "usage": {
            "input_tokens": 0,
            "output_tokens": 0,
            "total_tokens": 0,
        },
        "error": Value::Null,
    })
}

fn extract_input_prompt(input: Option<&Value>) -> Option<String> {
    let input = input?;

    if let Some(text) = input.as_str() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
        return None;
    }

    if let Some(array) = input.as_array() {
        let parts = array
            .iter()
            .filter_map(extract_prompt_from_item)
            .filter(|value| !value.trim().is_empty())
            .collect::<Vec<_>>();

        if parts.is_empty() {
            return None;
        }

        return Some(parts.join("\n"));
    }

    extract_prompt_from_item(input).and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn extract_prompt_from_item(item: &Value) -> Option<String> {
    if let Some(text) = item.as_str() {
        return Some(text.to_owned());
    }

    let object = item.as_object()?;
    if let Some(content) = object.get("content") {
        let text = extract_text_content(content);
        if !text.trim().is_empty() {
            return Some(text);
        }
    }

    if let Some(text) = object.get("text").and_then(Value::as_str) {
        return Some(text.to_owned());
    }

    if let Some(text) = object.get("input_text").and_then(Value::as_str) {
        return Some(text.to_owned());
    }

    None
}

fn resolve_openresponses_session_key(model: &str, user: Option<&str>) -> String {
    let agent_id = normalize_segment(model);
    let conversation = user
        .map(normalize_segment)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "default".to_owned());

    format!(
        "agent:{}:openresponses:chat:{conversation}",
        if agent_id.is_empty() {
            "main"
        } else {
            &agent_id
        }
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{extract_input_prompt, extract_prompt_from_item};

    #[test]
    fn extract_input_prompt_handles_string_and_array_payloads() {
        assert_eq!(
            extract_input_prompt(Some(&json!("hello"))).as_deref(),
            Some("hello")
        );
        assert_eq!(
            extract_input_prompt(Some(&json!([
                {"content": [{"type": "input_text", "text": "first"}]},
                {"text": "second"}
            ])))
            .as_deref(),
            Some("first\nsecond")
        );
    }

    #[test]
    fn extract_prompt_from_item_prefers_content() {
        let item = json!({
            "content": [{"type": "text", "text": "from content"}],
            "text": "fallback"
        });
        assert_eq!(
            extract_prompt_from_item(&item).as_deref(),
            Some("from content")
        );
    }
}
