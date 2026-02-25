use axum::{
    Json,
    extract::{Path, State},
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInboundRequest {
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
    ingress_response(&state, &headers, payload).await
}

pub async fn inbound_channel_handler(
    Path(channel): Path<String>,
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<ChannelInboundRequest>,
) -> impl IntoResponse {
    let inbound = InboundMessageRequest {
        channel,
        conversation_id: payload.conversation_id,
        text: payload.text,
        agent_id: payload.agent_id,
        sender_id: payload.sender_id,
        message_id: payload.message_id,
        idempotency_key: payload.idempotency_key,
        metadata: payload.metadata,
    };
    ingress_response(&state, &headers, inbound).await
}

#[derive(Debug)]
struct NormalizedInbound {
    channel: String,
    text: String,
    session_key: String,
    idempotency_key: String,
}

#[derive(Debug)]
pub struct InboundProcessResult {
    pub session_key: String,
    pub run_id: Option<String>,
    pub reply: Option<String>,
}

pub async fn ingest_inbound_message(
    state: &SharedState,
    payload: InboundMessageRequest,
) -> Result<InboundProcessResult, crate::protocol::ErrorShape> {
    let inbound = normalize_inbound(payload).map_err(|message| {
        crate::protocol::ErrorShape::new(crate::protocol::ERROR_INVALID_REQUEST, message)
    })?;

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
    let payload = methods::chat::handle_send(state, &session, Some(&params)).await?;

    let run_id = payload
        .get("runId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let reply = payload
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned);

    Ok(InboundProcessResult {
        session_key: params
            .get("sessionKey")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        run_id,
        reply,
    })
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

async fn ingress_response(
    state: &SharedState,
    headers: &HeaderMap,
    payload: InboundMessageRequest,
) -> (StatusCode, Json<Value>) {
    if let Some(required_token) = &state.config().channels_inbound_token
        && !has_bearer_token(headers, required_token)
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

    match ingest_inbound_message(state, payload).await {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "sessionKey": result.session_key,
                "runId": result.run_id,
                "reply": result.reply,
            })),
        ),
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

#[cfg(test)]
mod tests {
    use super::normalize_segment;

    #[test]
    fn normalize_segment_preserves_alphanumeric_shape() {
        assert_eq!(normalize_segment("Telegram Chat 123"), "telegram-chat-123");
        assert_eq!(normalize_segment("###"), "");
    }
}
