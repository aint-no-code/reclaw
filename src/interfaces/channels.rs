use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    application::state::SharedState,
    rpc::{SessionContext, methods, policy},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboundMessageRequest {
    pub channel: String,
    pub conversation_id: String,
    pub text: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub sender_id: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

pub async fn inbound_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<InboundMessageRequest>,
) -> impl IntoResponse {
    if let Some(required_token) = &state.config().channels_inbound_token
        && !has_bearer_token(&headers, required_token)
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "ok": false,
                "error": {
                    "code": "UNAUTHORIZED",
                    "message": "invalid or missing bearer token",
                }
            })),
        );
    }

    let inbound = match normalize_inbound(payload) {
        Ok(inbound) => inbound,
        Err(message) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "ok": false,
                    "error": {
                        "code": "INVALID_REQUEST",
                        "message": message,
                    }
                })),
            );
        }
    };

    let session = SessionContext {
        conn_id: format!("http-inbound-{}", uuid::Uuid::new_v4()),
        role: "operator".to_owned(),
        scopes: policy::default_operator_scopes(),
        client_id: format!("{}-bridge", inbound.channel),
        client_mode: "channel-bridge".to_owned(),
    };

    let params = json!({
        "sessionKey": inbound.session_key,
        "message": inbound.text,
        "idempotencyKey": inbound.idempotency_key,
    });
    let result = methods::chat::handle_send(&state, &session, Some(&params)).await;

    match result {
        Ok(payload) => {
            let reply = payload
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned);
            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "sessionKey": params["sessionKey"],
                    "runId": payload.get("runId"),
                    "reply": reply,
                })),
            )
        }
        Err(error) => {
            let status = if error.code == crate::protocol::ERROR_INVALID_REQUEST {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            (
                status,
                Json(json!({
                    "ok": false,
                    "error": error,
                })),
            )
        }
    }
}

#[derive(Debug)]
struct NormalizedInbound {
    channel: String,
    text: String,
    session_key: String,
    idempotency_key: String,
}

fn normalize_inbound(input: InboundMessageRequest) -> Result<NormalizedInbound, String> {
    let channel = normalize_segment(&input.channel);
    if channel.is_empty() {
        return Err("channel is required".to_owned());
    }

    let conversation = normalize_segment(&input.conversation_id);
    if conversation.is_empty() {
        return Err("conversationId is required".to_owned());
    }

    let agent_id = input
        .agent_id
        .map(|value| normalize_segment(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "main".to_owned());

    let text = input.text.trim().to_owned();
    if text.is_empty() {
        return Err("text is required".to_owned());
    }

    let _ = input.sender_id;
    let _ = input.metadata;

    let message_part = input
        .message_id
        .map(|value| normalize_segment(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let idempotency_key = input
        .idempotency_key
        .map(|value| normalize_segment(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("{channel}-{conversation}-{message_part}"));

    Ok(NormalizedInbound {
        channel: channel.clone(),
        text,
        session_key: format!("agent:{agent_id}:{channel}:chat:{conversation}"),
        idempotency_key,
    })
}

fn normalize_segment(value: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;

    for ch in value.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            out.push(lower);
            pending_dash = false;
            continue;
        }

        if lower == '_' || lower == '-' || lower == ':' || lower.is_ascii_whitespace() {
            pending_dash = true;
        }
    }

    out.trim_matches('-').to_owned()
}

fn has_bearer_token(headers: &HeaderMap, expected: &str) -> bool {
    let Some(header_value) = headers.get(header::AUTHORIZATION) else {
        return false;
    };
    let Ok(auth) = header_value.to_str() else {
        return false;
    };

    let Some(token) = auth.strip_prefix("Bearer ") else {
        return false;
    };

    subtle::ConstantTimeEq::ct_eq(token.as_bytes(), expected.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::normalize_segment;

    #[test]
    fn normalize_segment_preserves_alphanumeric_shape() {
        assert_eq!(normalize_segment("Telegram Chat 123"), "telegram-chat-123");
        assert_eq!(normalize_segment("###"), "");
    }
}
