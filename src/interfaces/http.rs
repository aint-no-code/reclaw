use std::{future::Future, net::SocketAddr};

use axum::routing::post;
use axum::{
    Json, Router,
    extract::{Extension, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use tokio::net::TcpListener;
use tracing::info;

use crate::{
    application::state::SharedState,
    domain::error::DomainError,
    interfaces::{
        channels, hooks, openai, openresponses, slack_http, telegram, tools_invoke, webhooks, ws,
    },
    rpc::methods::{health, status},
};

pub fn build_router(state: SharedState) -> Router {
    build_router_with_webhooks(state, webhooks::default_registry())
}

pub fn build_router_with_webhooks(
    state: SharedState,
    webhook_registry: webhooks::ChannelWebhookRegistry,
) -> Router {
    let mut router = Router::new()
        .route("/", get(ws::ws_handler))
        .route("/ws", get(ws::ws_handler))
        .route("/healthz", get(healthz_handler))
        .route("/readyz", get(readyz_handler))
        .route("/info", get(info_handler))
        .route("/tools/invoke", post(tools_invoke::invoke_handler))
        .route("/slack/events", post(slack_http::events_handler))
        .route("/channels/inbound", post(channels::inbound_handler))
        .route(
            "/channels/{channel}/inbound",
            post(channels::inbound_channel_handler),
        )
        .route(
            "/channels/telegram/webhook",
            post(telegram::webhook_handler),
        )
        .route(
            "/channels/{channel}/webhook",
            post(webhooks::channel_webhook_handler),
        )
        .layer(Extension(webhook_registry));

    if state.config().hooks_enabled {
        let hooks_base_path = state.config().hooks_path.clone();
        let hooks_subpath = format!("{hooks_base_path}/{{*subpath}}");
        router = router
            .route(hooks_base_path.as_str(), post(hooks::root_handler))
            .route(hooks_subpath.as_str(), post(hooks::subpath_handler));
    }

    if state.config().openai_chat_completions_enabled {
        router = router.route(
            "/v1/chat/completions",
            post(openai::chat_completions_handler),
        );
    }

    if state.config().openresponses_enabled {
        router = router.route("/v1/responses", post(openresponses::responses_handler));
    }

    router.with_state(state)
}

pub async fn serve(
    listener: TcpListener,
    state: SharedState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), DomainError> {
    let local_addr = listener.local_addr().map_err(|error| {
        DomainError::Unavailable(format!("failed to read listener address: {error}"))
    })?;

    info!(
        "reclaw-core listening on ws://{}:{}, auth_mode={}, protocol={}",
        local_addr.ip(),
        local_addr.port(),
        state.auth_mode_label(),
        crate::protocol::PROTOCOL_VERSION,
    );

    axum::serve(
        listener,
        build_router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    .map_err(|error| DomainError::Unavailable(format!("server runtime error: {error}")))
}

async fn healthz_handler(State(state): State<SharedState>) -> impl IntoResponse {
    match state.health_payload().await {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "ok": false,
                "error": error.to_string(),
            })),
        )
            .into_response(),
    }
}

async fn readyz_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let connections = state.connection_count().await;
    let payload = health::ready_payload(&state, connections);
    (StatusCode::OK, Json(payload))
}

async fn info_handler(State(state): State<SharedState>) -> impl IntoResponse {
    let payload = status::info_payload(&state);
    (StatusCode::OK, Json(payload))
}
