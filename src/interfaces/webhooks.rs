use std::{collections::HashMap, future::Future, pin::Pin};

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde_json::{Value, json};

use crate::application::state::SharedState;

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
    let Some(adapter) = registry.adapter_for(&channel) else {
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

    adapter(&state, &headers, payload).await
}

#[cfg(test)]
mod tests {
    use super::{
        ChannelWebhookAdapter, ChannelWebhookRegistry, DISCORD_ADAPTER, SIGNAL_ADAPTER,
        SLACK_ADAPTER, TELEGRAM_ADAPTER, WHATSAPP_ADAPTER, WebhookFuture,
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
}
