use axum::http::HeaderMap;
use serde_json::{Value, json};

use crate::application::state::SharedState;

use super::{channel_adapter_common as common, webhooks::WebhookFuture};

const DISCORD_EVENTS_PREFIX: &str = "runtime/discord/event/";

pub(crate) fn dispatch_webhook<'a>(
    state: &'a SharedState,
    headers: &'a HeaderMap,
    payload: Value,
) -> WebhookFuture<'a> {
    Box::pin(async move {
        if let Err(error) = common::require_channel_bearer_token(
            headers,
            &state.config().discord_webhook_token,
            "discord",
        ) {
            return error;
        }

        let data = payload.get("d").unwrap_or(&payload);

        let conversation_id = read_string(data, "channel_id").unwrap_or_default();
        if conversation_id.trim().is_empty() {
            return common::accepted_false("no-channel");
        }

        let text = read_string(data, "content").unwrap_or_default();
        let text = text.trim().to_owned();
        if text.is_empty() {
            return common::accepted_false("no-text");
        }

        let message_id = read_string(data, "id").unwrap_or_default();
        if message_id.trim().is_empty() {
            return common::accepted_false("no-message-id");
        }

        let dedupe_key = format!("{DISCORD_EVENTS_PREFIX}{message_id}");
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

        let sender_id = data
            .get("author")
            .and_then(|author| author.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let outbound_conversation_id = conversation_id.clone();
        let outbound_sender_id = sender_id.clone();

        let result = match common::ingest_channel_message(
            state,
            common::ChannelInboundEvent {
                channel: "discord",
                conversation_id,
                text,
                sender_id,
                message_id: Some(message_id.clone()),
                idempotency_key: format!("discord-{message_id}"),
                metadata: Some(json!({
                    "source": "discord",
                })),
            },
        )
        .await
        {
            Ok(result) => result,
            Err(error) => return error,
        };

        common::mark_event_processed(state, &dedupe_key, "discord", &message_id, &result).await;
        let outbound_sent = common::maybe_dispatch_outbound_reply(
            state,
            state.config().discord_outbound_url.as_deref(),
            state.config().discord_outbound_token.as_deref(),
            common::OutboundReplyDispatch {
                channel: "discord",
                conversation_id: &outbound_conversation_id,
                source_sender_id: outbound_sender_id.as_deref(),
                source_message_id: Some(message_id.as_str()),
                reply: result.reply.as_deref(),
                session_key: &result.session_key,
                run_id: result.run_id.as_deref(),
                metadata: Some(json!({
                    "source": "discord",
                })),
                log_scope: "channels.discord.webhook",
            },
        )
        .await;

        common::accepted_true_with_outbound(&result, outbound_sent)
    })
}

fn read_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}
