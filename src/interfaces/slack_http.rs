use axum::{
    Json,
    extract::{Extension, Path, State},
    http::HeaderMap,
    response::IntoResponse,
};
use serde_json::Value;

use crate::application::state::SharedState;

use super::webhooks::{self, ChannelWebhookRegistry};

pub async fn events_handler(
    State(state): State<SharedState>,
    Extension(registry): Extension<ChannelWebhookRegistry>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    webhooks::channel_webhook_handler(
        Path("slack".to_owned()),
        State(state),
        Extension(registry),
        headers,
        Json(payload),
    )
    .await
}
