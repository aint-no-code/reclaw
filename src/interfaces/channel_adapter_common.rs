use std::time::Duration;

use axum::{
    Json,
    http::{HeaderMap, StatusCode, header},
};
use serde_json::{Value, json};
use tracing::warn;

use crate::{
    application::state::SharedState,
    interfaces::channels::{InboundMessageRequest, InboundProcessResult, ingest_inbound_message},
    storage::now_unix_ms,
};

pub(crate) struct ChannelInboundEvent {
    pub channel: &'static str,
    pub conversation_id: String,
    pub text: String,
    pub sender_id: Option<String>,
    pub message_id: Option<String>,
    pub idempotency_key: String,
    pub metadata: Option<Value>,
}

pub(crate) fn require_channel_bearer_token(
    headers: &HeaderMap,
    token: &Option<String>,
    channel: &str,
) -> Result<(), (StatusCode, Json<Value>)> {
    let Some(token) = token.as_deref() else {
        return Err(unavailable(format!(
            "{channel} webhook token is not configured"
        )));
    };

    if !has_bearer_token(headers, token) {
        return Err(unauthorized("invalid or missing bearer token"));
    }

    Ok(())
}

pub(crate) fn unauthorized(message: impl Into<String>) -> (StatusCode, Json<Value>) {
    error_response(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", message)
}

pub(crate) fn unavailable(message: impl Into<String>) -> (StatusCode, Json<Value>) {
    error_response(StatusCode::SERVICE_UNAVAILABLE, "UNAVAILABLE", message)
}

pub(crate) fn bad_request(message: impl Into<String>) -> (StatusCode, Json<Value>) {
    error_response(StatusCode::BAD_REQUEST, "INVALID_REQUEST", message)
}

pub(crate) fn accepted_false(reason: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "accepted": false,
            "reason": reason.into(),
        })),
    )
}

pub(crate) async fn ingest_channel_message(
    state: &SharedState,
    event: ChannelInboundEvent,
) -> Result<InboundProcessResult, (StatusCode, Json<Value>)> {
    let inbound = InboundMessageRequest {
        channel: event.channel.to_owned(),
        conversation_id: event.conversation_id,
        text: event.text,
        agent_id: Some("main".to_owned()),
        sender_id: event.sender_id,
        message_id: event.message_id,
        idempotency_key: Some(event.idempotency_key),
        metadata: event.metadata,
    };

    let result = ingest_inbound_message(state, inbound).await;
    match result {
        Ok(result) => Ok(result),
        Err(error) => {
            let status = if error.code == crate::protocol::ERROR_INVALID_REQUEST {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            Err((
                status,
                Json(json!({
                    "ok": false,
                    "error": error,
                })),
            ))
        }
    }
}

pub(crate) struct OutboundReplyDispatch<'a> {
    pub channel: &'static str,
    pub conversation_id: &'a str,
    pub source_sender_id: Option<&'a str>,
    pub source_message_id: Option<&'a str>,
    pub reply: Option<&'a str>,
    pub session_key: &'a str,
    pub run_id: Option<&'a str>,
    pub metadata: Option<Value>,
    pub log_scope: &'static str,
}

pub(crate) async fn maybe_dispatch_outbound_reply(
    state: &SharedState,
    outbound_url: Option<&str>,
    outbound_token: Option<&str>,
    dispatch: OutboundReplyDispatch<'_>,
) -> bool {
    let Some(reply) = dispatch
        .reply
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let Some(url) = outbound_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };

    let mut payload = json!({
        "channel": dispatch.channel,
        "conversationId": dispatch.conversation_id,
        "reply": reply,
        "sessionKey": dispatch.session_key,
        "runId": dispatch.run_id,
        "sourceSenderId": dispatch.source_sender_id,
        "sourceMessageId": dispatch.source_message_id,
    });
    if let Some(metadata) = dispatch.metadata
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("metadata".to_owned(), metadata);
    }

    match post_json(url, outbound_token, &payload).await {
        Ok(()) => true,
        Err(error) => {
            warn!(
                "{} outbound relay failed for channel {}: {}",
                dispatch.log_scope, dispatch.channel, error
            );
            let _ = state
                .append_gateway_log(
                    "warn",
                    &format!(
                        "{} outbound relay failed for channel {}: {}",
                        dispatch.log_scope, dispatch.channel, error
                    ),
                    Some(dispatch.log_scope),
                    None,
                )
                .await;
            false
        }
    }
}

pub(crate) fn accepted_true_with_outbound(
    result: &InboundProcessResult,
    outbound_sent: bool,
) -> (StatusCode, Json<Value>) {
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "accepted": true,
            "sessionKey": result.session_key,
            "runId": result.run_id,
            "reply": result.reply,
            "outboundSent": outbound_sent,
        })),
    )
}

pub(crate) async fn is_duplicate_event(state: &SharedState, key: &str) -> bool {
    state
        .get_config_entry_value(key)
        .await
        .ok()
        .flatten()
        .is_some()
}

pub(crate) async fn mark_event_processed(
    state: &SharedState,
    key: &str,
    channel: &str,
    event_id: &str,
    result: &InboundProcessResult,
) {
    let _ = state
        .set_config_entry_value(
            key,
            &json!({
                "processedAtMs": now_unix_ms(),
                "channel": channel,
                "eventId": event_id,
                "sessionKey": result.session_key,
                "runId": result.run_id,
            }),
        )
        .await;
}

fn error_response(
    status: StatusCode,
    code: &str,
    message: impl Into<String>,
) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({
            "ok": false,
            "error": {
                "code": code,
                "message": message.into(),
            }
        })),
    )
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

async fn post_json(url: &str, token: Option<&str>, payload: &Value) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| format!("failed to construct http client: {error}"))?;

    let mut request = client.post(url).json(payload);
    if let Some(token) = token.map(str::trim).filter(|value| !value.is_empty()) {
        request = request.bearer_auth(token);
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("http request failed: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("unexpected status {status}: {body}"));
    }

    Ok(())
}
