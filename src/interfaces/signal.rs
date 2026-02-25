use axum::http::HeaderMap;
use serde_json::{Value, json};

use crate::application::state::SharedState;

use super::{channel_adapter_common as common, webhooks::WebhookFuture};

const SIGNAL_EVENTS_PREFIX: &str = "runtime/signal/event/";

pub(crate) fn dispatch_webhook<'a>(
    state: &'a SharedState,
    headers: &'a HeaderMap,
    payload: Value,
) -> WebhookFuture<'a> {
    Box::pin(async move {
        if let Err(error) = common::require_channel_bearer_token(
            headers,
            &state.config().signal_webhook_token,
            "signal",
        ) {
            return error;
        }

        let Some(envelope) = payload.get("envelope") else {
            return common::accepted_false("no-envelope");
        };

        let text = envelope
            .get("dataMessage")
            .and_then(|data| data.get("message"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if text.is_empty() {
            return common::accepted_false("no-text");
        }

        let conversation_id = envelope
            .get("sourceNumber")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if conversation_id.is_empty() {
            return common::accepted_false("no-source");
        }

        let timestamp = envelope
            .get("timestamp")
            .and_then(Value::as_i64)
            .map(|value| value.to_string())
            .unwrap_or_default();
        if timestamp.is_empty() {
            return common::accepted_false("no-timestamp");
        }

        let dedupe_key = format!("{SIGNAL_EVENTS_PREFIX}{timestamp}");
        if common::is_duplicate_event(state, &dedupe_key).await {
            return (
                axum::http::StatusCode::OK,
                axum::Json(json!({
                    "ok": true,
                    "accepted": false,
                    "duplicate": true,
                })),
            );
        }

        let outbound_conversation_id = conversation_id.clone();
        let result = match common::ingest_channel_message(
            state,
            common::ChannelInboundEvent {
                channel: "signal",
                conversation_id: conversation_id.clone(),
                text,
                sender_id: Some(conversation_id),
                message_id: Some(timestamp.clone()),
                idempotency_key: format!("signal-{timestamp}"),
                metadata: Some(json!({
                    "source": "signal",
                })),
            },
        )
        .await
        {
            Ok(result) => result,
            Err(error) => return error,
        };

        common::mark_event_processed(state, &dedupe_key, "signal", &timestamp, &result).await;
        let outbound_sent = common::maybe_dispatch_outbound_reply(
            state,
            state.config().signal_outbound_url.as_deref(),
            state.config().signal_outbound_token.as_deref(),
            common::OutboundReplyDispatch {
                channel: "signal",
                conversation_id: &outbound_conversation_id,
                source_sender_id: Some(outbound_conversation_id.as_str()),
                source_message_id: Some(timestamp.as_str()),
                reply: result.reply.as_deref(),
                session_key: &result.session_key,
                run_id: result.run_id.as_deref(),
                metadata: Some(json!({
                    "source": "signal",
                })),
                log_scope: "channels.signal.webhook",
            },
        )
        .await;

        common::accepted_true_with_outbound(&result, outbound_sent)
    })
}
