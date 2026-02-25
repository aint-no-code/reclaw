use std::{future::Future, pin::Pin, time::Duration};

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::warn;

use crate::{application::state::SharedState, interfaces::channels};

const TELEGRAM_SECRET_HEADER: &str = "x-telegram-bot-api-secret-token";
const TELEGRAM_UPDATES_PREFIX: &str = "runtime/telegram/update/";

#[derive(Debug, Deserialize)]
pub struct TelegramWebhookUpdate {
    #[serde(rename = "update_id", alias = "updateId")]
    pub update_id: i64,
    #[serde(default)]
    pub message: Option<TelegramMessage>,
    #[serde(default)]
    #[serde(rename = "edited_message", alias = "editedMessage")]
    pub edited_message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramMessage {
    #[serde(rename = "message_id", alias = "messageId")]
    pub message_id: i64,
    pub chat: TelegramChat,
    #[serde(default)]
    pub from: Option<TelegramUser>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramChat {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramUser {
    pub id: i64,
}

#[derive(Debug, Serialize)]
struct TelegramSendMessageBody {
    chat_id: i64,
    text: String,
}

pub async fn webhook_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(update): Json<TelegramWebhookUpdate>,
) -> impl IntoResponse {
    handle_webhook_update(&state, &headers, update).await
}

pub(crate) fn dispatch_webhook<'a>(
    state: &'a SharedState,
    headers: &'a HeaderMap,
    payload: Value,
) -> Pin<Box<dyn Future<Output = (StatusCode, Json<Value>)> + Send + 'a>> {
    Box::pin(async move {
        let update = match serde_json::from_value::<TelegramWebhookUpdate>(payload) {
            Ok(update) => update,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "ok": false,
                        "error": {
                            "code": "INVALID_REQUEST",
                            "message": format!("invalid telegram webhook payload: {error}"),
                        }
                    })),
                );
            }
        };

        handle_webhook_update(state, headers, update).await
    })
}

async fn handle_webhook_update(
    state: &SharedState,
    headers: &HeaderMap,
    update: TelegramWebhookUpdate,
) -> (StatusCode, Json<Value>) {
    let Some(secret) = &state.config().telegram_webhook_secret else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "ok": false,
                "error": {
                    "code": "UNAVAILABLE",
                    "message": "telegram webhook is not configured",
                }
            })),
        );
    };

    if !valid_telegram_secret(headers, secret) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "ok": false,
                "error": {
                    "code": "UNAUTHORIZED",
                    "message": "invalid telegram webhook secret",
                }
            })),
        );
    }

    let Some(message) = update.message.or(update.edited_message) else {
        return (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "accepted": false,
                "reason": "no-message",
            })),
        );
    };

    let text = message
        .text
        .or(message.caption)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let Some(text) = text else {
        return (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "accepted": false,
                "reason": "no-text",
            })),
        );
    };

    let dedupe_key = format!("{TELEGRAM_UPDATES_PREFIX}{}", update.update_id);
    if state
        .get_config_entry_value(&dedupe_key)
        .await
        .ok()
        .flatten()
        .is_some()
    {
        return (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "accepted": false,
                "duplicate": true,
            })),
        );
    }

    let inbound = channels::InboundMessageRequest {
        channel: "telegram".to_owned(),
        conversation_id: message.chat.id.to_string(),
        text,
        agent_id: Some("main".to_owned()),
        sender_id: message.from.map(|user| user.id.to_string()),
        message_id: Some(message.message_id.to_string()),
        idempotency_key: Some(format!("telegram-{}", update.update_id)),
        metadata: Some(json!({
            "updateId": update.update_id,
        })),
    };

    let result = channels::ingest_inbound_message(state, inbound).await;
    let result = match result {
        Ok(value) => value,
        Err(error) => {
            let status = if error.code == crate::protocol::ERROR_INVALID_REQUEST {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            return (
                status,
                Json(json!({
                    "ok": false,
                    "error": error,
                })),
            );
        }
    };

    let _ = state
        .set_config_entry_value(
            &dedupe_key,
            &json!({
                "processedAtMs": crate::storage::now_unix_ms(),
                "sessionKey": result.session_key,
                "runId": result.run_id,
            }),
        )
        .await;

    let mut outbound_sent = false;
    if let (Some(bot_token), Some(reply)) = (&state.config().telegram_bot_token, &result.reply) {
        match send_telegram_message(state, bot_token, message.chat.id, reply).await {
            Ok(()) => outbound_sent = true,
            Err(error) => {
                warn!("telegram outbound send failed: {error}");
                let _ = state
                    .append_gateway_log(
                        "warn",
                        &format!("telegram outbound send failed: {error}"),
                        Some("channels.telegram.webhook"),
                        None,
                    )
                    .await;
            }
        }
    }

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

async fn send_telegram_message(
    state: &SharedState,
    bot_token: &str,
    chat_id: i64,
    text: &str,
) -> Result<(), String> {
    let base_url = state.config().telegram_api_base_url.trim_end_matches('/');
    let url = format!("{base_url}/bot{bot_token}/sendMessage");
    let body = TelegramSendMessageBody {
        chat_id,
        text: text.to_owned(),
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| format!("failed to construct http client: {error}"))?;

    let response = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("telegram request failed: {error}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("telegram send failed with {status}: {body}"));
    }

    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("telegram response decode failed: {error}"))?;

    if !payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return Err(format!("telegram API returned failure payload: {payload}"));
    }

    Ok(())
}

fn valid_telegram_secret(headers: &HeaderMap, expected: &str) -> bool {
    let Some(header_value) = headers.get(TELEGRAM_SECRET_HEADER) else {
        return false;
    };
    let Ok(found) = header_value.to_str() else {
        return false;
    };

    subtle::ConstantTimeEq::ct_eq(found.as_bytes(), expected.as_bytes()).into()
}
