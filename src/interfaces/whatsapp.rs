use axum::http::HeaderMap;
use serde_json::{Value, json};

use crate::application::state::SharedState;

use super::{channel_adapter_common as common, webhooks::WebhookFuture};

const WHATSAPP_EVENTS_PREFIX: &str = "runtime/whatsapp/event/";

pub(crate) fn dispatch_webhook<'a>(
    state: &'a SharedState,
    headers: &'a HeaderMap,
    payload: Value,
) -> WebhookFuture<'a> {
    Box::pin(async move {
        if let Err(error) = common::require_channel_bearer_token(
            headers,
            &state.config().whatsapp_webhook_token,
            "whatsapp",
        ) {
            return error;
        }

        let Some(message) = first_whatsapp_message(&payload) else {
            return common::accepted_false("no-message");
        };

        let text = message
            .get("text")
            .and_then(|text| text.get("body"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if text.is_empty() {
            return common::accepted_false("no-text");
        }

        let from = message
            .get("from")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if from.is_empty() {
            return common::accepted_false("no-from");
        }

        let message_id = message
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if message_id.is_empty() {
            return common::accepted_false("no-message-id");
        }

        let dedupe_key = format!("{WHATSAPP_EVENTS_PREFIX}{message_id}");
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

        let result = match common::ingest_channel_message(
            state,
            common::ChannelInboundEvent {
                channel: "whatsapp",
                conversation_id: from.clone(),
                text,
                sender_id: Some(from),
                message_id: Some(message_id.clone()),
                idempotency_key: format!("whatsapp-{message_id}"),
                metadata: Some(json!({
                    "source": "whatsapp",
                })),
            },
        )
        .await
        {
            Ok(result) => result,
            Err(error) => return error,
        };

        common::mark_event_processed(state, &dedupe_key, "whatsapp", &message_id, &result).await;
        common::accepted_true(&result)
    })
}

fn first_whatsapp_message(payload: &Value) -> Option<&Value> {
    payload
        .get("entry")?
        .as_array()?
        .first()?
        .get("changes")?
        .as_array()?
        .first()?
        .get("value")?
        .get("messages")?
        .as_array()?
        .first()
}
