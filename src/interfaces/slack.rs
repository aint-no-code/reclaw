use axum::{Json, http::HeaderMap};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::state::SharedState;

use super::{channel_adapter_common as common, webhooks::WebhookFuture};

const SLACK_EVENTS_PREFIX: &str = "runtime/slack/event/";

#[derive(Debug, Deserialize)]
struct SlackWebhookPayload {
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    challenge: Option<String>,
    #[serde(default)]
    event_id: Option<String>,
    #[serde(default)]
    event: Option<SlackEvent>,
}

#[derive(Debug, Deserialize)]
struct SlackEvent {
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    ts: Option<String>,
}

pub(crate) fn dispatch_webhook<'a>(
    state: &'a SharedState,
    headers: &'a HeaderMap,
    payload: Value,
) -> WebhookFuture<'a> {
    Box::pin(async move {
        if let Err(error) = common::require_channel_bearer_token(
            headers,
            &state.config().slack_webhook_token,
            "slack",
        ) {
            return error;
        }

        let payload = match serde_json::from_value::<SlackWebhookPayload>(payload) {
            Ok(payload) => payload,
            Err(error) => {
                return common::bad_request(format!("invalid slack webhook payload: {error}"));
            }
        };

        if payload.r#type.as_deref() == Some("url_verification")
            && let Some(challenge) = payload.challenge
        {
            return (
                axum::http::StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "challenge": challenge,
                })),
            );
        }

        let Some(event) = payload.event else {
            return common::accepted_false("no-event");
        };
        if event.r#type.as_deref() != Some("message") {
            return common::accepted_false("unsupported-event");
        }

        let conversation_id = event.channel.unwrap_or_default().trim().to_owned();
        if conversation_id.is_empty() {
            return common::accepted_false("no-channel");
        }

        let text = event.text.unwrap_or_default().trim().to_owned();
        if text.is_empty() {
            return common::accepted_false("no-text");
        }

        let dedupe_id = payload
            .event_id
            .unwrap_or_else(|| event.ts.clone().unwrap_or_default());
        if dedupe_id.trim().is_empty() {
            return common::accepted_false("no-event-id");
        }

        let dedupe_key = format!("{SLACK_EVENTS_PREFIX}{dedupe_id}");
        if common::is_duplicate_event(state, &dedupe_key).await {
            return (
                axum::http::StatusCode::OK,
                Json(json!({
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
                channel: "slack",
                conversation_id,
                text,
                sender_id: event.user,
                message_id: event.ts.clone(),
                idempotency_key: format!("slack-{dedupe_id}"),
                metadata: Some(json!({
                    "eventType": "message",
                    "eventTs": event.ts,
                })),
            },
        )
        .await
        {
            Ok(result) => result,
            Err(error) => return error,
        };

        common::mark_event_processed(state, &dedupe_key, "slack", &dedupe_id, &result).await;
        let outbound_sent = common::maybe_dispatch_outbound_reply(
            state,
            state.config().slack_outbound_url.as_deref(),
            state.config().slack_outbound_token.as_deref(),
            common::OutboundReplyDispatch {
                channel: "slack",
                conversation_id: &outbound_conversation_id,
                source_sender_id: None,
                source_message_id: Some(dedupe_id.as_str()),
                reply: result.reply.as_deref(),
                session_key: &result.session_key,
                run_id: result.run_id.as_deref(),
                metadata: Some(json!({
                    "eventType": "message",
                    "eventId": dedupe_id,
                })),
                log_scope: "channels.slack.webhook",
            },
        )
        .await;

        common::accepted_true_with_outbound(&result, outbound_sent)
    })
}
