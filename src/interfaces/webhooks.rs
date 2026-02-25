use std::{future::Future, pin::Pin};

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde_json::{Value, json};

use crate::application::state::SharedState;

use super::telegram;

type WebhookFuture<'a> = Pin<Box<dyn Future<Output = (StatusCode, Json<Value>)> + Send + 'a>>;
type WebhookDispatchFn = for<'a> fn(&'a SharedState, &'a HeaderMap, Value) -> WebhookFuture<'a>;

#[derive(Clone, Copy)]
struct ChannelWebhookAdapter {
    channel: &'static str,
    dispatch: WebhookDispatchFn,
}

const TELEGRAM_ADAPTER: ChannelWebhookAdapter = ChannelWebhookAdapter {
    channel: "telegram",
    dispatch: telegram::dispatch_webhook,
};

const ADAPTERS: &[ChannelWebhookAdapter] = &[TELEGRAM_ADAPTER];

pub async fn channel_webhook_handler(
    Path(channel): Path<String>,
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let Some(adapter) = adapter_for(&channel) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "error": {
                    "code": "NOT_FOUND",
                    "message": "unknown channel webhook adapter",
                }
            })),
        );
    };

    (adapter.dispatch)(&state, &headers, payload).await
}

fn adapter_for(channel: &str) -> Option<&'static ChannelWebhookAdapter> {
    ADAPTERS.iter().find(|adapter| adapter.channel == channel)
}
