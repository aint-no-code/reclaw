use std::{collections::HashMap, future::Future, pin::Pin, time::Duration};

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde_json::{Value, json};

use crate::application::{config::ChannelWebhookPluginConfig, state::SharedState};

use super::{discord, signal, slack, telegram, whatsapp};

pub type WebhookFuture<'a> = Pin<Box<dyn Future<Output = (StatusCode, Json<Value>)> + Send + 'a>>;
pub type WebhookDispatchFn = for<'a> fn(&'a SharedState, &'a HeaderMap, Value) -> WebhookFuture<'a>;

#[derive(Clone, Copy)]
pub struct ChannelWebhookAdapter {
    pub channel: &'static str,
    pub dispatch: WebhookDispatchFn,
}

pub const TELEGRAM_ADAPTER: ChannelWebhookAdapter = ChannelWebhookAdapter {
    channel: "telegram",
    dispatch: telegram::dispatch_webhook,
};
pub const DISCORD_ADAPTER: ChannelWebhookAdapter = ChannelWebhookAdapter {
    channel: "discord",
    dispatch: discord::dispatch_webhook,
};
pub const SLACK_ADAPTER: ChannelWebhookAdapter = ChannelWebhookAdapter {
    channel: "slack",
    dispatch: slack::dispatch_webhook,
};
pub const SIGNAL_ADAPTER: ChannelWebhookAdapter = ChannelWebhookAdapter {
    channel: "signal",
    dispatch: signal::dispatch_webhook,
};
pub const WHATSAPP_ADAPTER: ChannelWebhookAdapter = ChannelWebhookAdapter {
    channel: "whatsapp",
    dispatch: whatsapp::dispatch_webhook,
};

const CHANNEL_PLUGIN_TOKEN_HEADER: &str = "x-reclaw-plugin-token";
const CHANNEL_PLUGIN_NAME_HEADER: &str = "x-reclaw-channel";

#[derive(Clone, Default)]
pub struct ChannelWebhookRegistry {
    adapters: HashMap<String, WebhookDispatchFn>,
}

impl ChannelWebhookRegistry {
    #[must_use]
    pub fn with_adapters(adapters: &[ChannelWebhookAdapter]) -> Self {
        let mut registry = Self::default();
        for adapter in adapters {
            registry.register(*adapter);
        }
        registry
    }

    pub fn register(&mut self, adapter: ChannelWebhookAdapter) {
        self.adapters
            .insert(adapter.channel.to_owned(), adapter.dispatch);
    }

    fn adapter_for(&self, channel: &str) -> Option<WebhookDispatchFn> {
        self.adapters.get(channel).copied()
    }
}

#[must_use]
pub fn default_registry() -> ChannelWebhookRegistry {
    ChannelWebhookRegistry::with_adapters(&[
        TELEGRAM_ADAPTER,
        DISCORD_ADAPTER,
        SLACK_ADAPTER,
        SIGNAL_ADAPTER,
        WHATSAPP_ADAPTER,
    ])
}

pub async fn channel_webhook_handler(
    Path(channel): Path<String>,
    State(state): State<SharedState>,
    Extension(registry): Extension<ChannelWebhookRegistry>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let channel_key =
        normalize_channel_key(&channel).unwrap_or_else(|| channel.to_ascii_lowercase());
    if let Some(adapter) = registry.adapter_for(&channel_key) {
        return adapter(&state, &headers, payload).await;
    }

    if let Some(plugin) = state.config().channel_webhook_plugins.get(&channel_key) {
        return proxy_channel_webhook(&channel_key, plugin, &headers, payload).await;
    }

    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "ok": false,
            "error": {
                "code": "NOT_FOUND",
                "message": "unknown channel webhook adapter",
            }
        })),
    )
}

fn normalize_channel_key(input: &str) -> Option<String> {
    let normalized = input.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    if normalized.chars().all(|ch| {
        ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_' || ch == '.'
    }) {
        Some(normalized)
    } else {
        None
    }
}

async fn proxy_channel_webhook(
    channel: &str,
    plugin: &ChannelWebhookPluginConfig,
    headers: &HeaderMap,
    payload: Value,
) -> (StatusCode, Json<Value>) {
    let timeout_ms = plugin.timeout_ms.unwrap_or(10_000);
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "ok": false,
                    "error": {
                        "code": "BAD_GATEWAY",
                        "message": format!("failed to initialize channel plugin client: {error}"),
                    }
                })),
            );
        }
    };

    let mut request = client
        .post(&plugin.url)
        .header(CHANNEL_PLUGIN_NAME_HEADER, channel)
        .json(&payload);
    if let Some(token) = plugin.token.as_deref() {
        request = request.header(CHANNEL_PLUGIN_TOKEN_HEADER, token);
    }
    for (name, value) in headers {
        if name == axum::http::header::HOST
            || name == axum::http::header::CONTENT_LENGTH
            || name
                .as_str()
                .eq_ignore_ascii_case(CHANNEL_PLUGIN_TOKEN_HEADER)
            || name
                .as_str()
                .eq_ignore_ascii_case(CHANNEL_PLUGIN_NAME_HEADER)
        {
            continue;
        }
        request = request.header(name, value);
    }

    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "ok": false,
                    "error": {
                        "code": "BAD_GATEWAY",
                        "message": format!("channel plugin request failed: {error}"),
                    }
                })),
            );
        }
    };
    let status = response.status();
    let body = match response.bytes().await {
        Ok(body) => body,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "ok": false,
                    "error": {
                        "code": "BAD_GATEWAY",
                        "message": format!("failed to read channel plugin response: {error}"),
                    }
                })),
            );
        }
    };
    let parsed = match serde_json::from_slice::<Value>(&body) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "ok": false,
                    "error": {
                        "code": "BAD_GATEWAY",
                        "message": format!("channel plugin response must be valid JSON: {error}"),
                    }
                })),
            );
        }
    };

    (status, Json(parsed))
}

#[cfg(test)]
mod tests {
    use super::{
        ChannelWebhookAdapter, ChannelWebhookRegistry, DISCORD_ADAPTER, SIGNAL_ADAPTER,
        SLACK_ADAPTER, TELEGRAM_ADAPTER, WHATSAPP_ADAPTER, WebhookFuture, normalize_channel_key,
    };
    use crate::application::state::SharedState;
    use axum::{
        Json,
        http::{HeaderMap, StatusCode},
    };
    use serde_json::{Value, json};

    fn fake_dispatch<'a>(
        _state: &'a SharedState,
        _headers: &'a HeaderMap,
        _payload: Value,
    ) -> WebhookFuture<'a> {
        Box::pin(async { (StatusCode::OK, Json(json!({ "ok": true }))) })
    }

    #[test]
    fn registry_resolves_registered_adapters() {
        let mut registry = ChannelWebhookRegistry::default();
        registry.register(ChannelWebhookAdapter {
            channel: "fake",
            dispatch: fake_dispatch,
        });

        assert!(registry.adapter_for("fake").is_some());
        assert!(registry.adapter_for("missing").is_none());
    }

    #[test]
    fn default_registry_includes_telegram() {
        let registry = ChannelWebhookRegistry::with_adapters(&[
            TELEGRAM_ADAPTER,
            DISCORD_ADAPTER,
            SLACK_ADAPTER,
            SIGNAL_ADAPTER,
            WHATSAPP_ADAPTER,
        ]);
        assert!(registry.adapter_for("telegram").is_some());
        assert!(registry.adapter_for("discord").is_some());
        assert!(registry.adapter_for("slack").is_some());
        assert!(registry.adapter_for("signal").is_some());
        assert!(registry.adapter_for("whatsapp").is_some());
    }

    #[test]
    fn normalize_channel_key_rejects_invalid_values() {
        assert_eq!(
            normalize_channel_key(" Telegram "),
            Some("telegram".to_owned())
        );
        assert_eq!(
            normalize_channel_key("bridge.chat"),
            Some("bridge.chat".to_owned())
        );
        assert!(normalize_channel_key("bad/channel").is_none());
        assert!(normalize_channel_key("").is_none());
    }
}
