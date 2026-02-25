use std::{future::Future, net::SocketAddr};

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use tokio::net::TcpListener;
use tracing::info;

use crate::{
    application::state::SharedState,
    domain::error::DomainError,
    interfaces::ws,
    rpc::methods::{health, status},
};

pub fn build_router(state: SharedState) -> Router {
    Router::new()
        .route("/", get(ws::ws_handler))
        .route("/ws", get(ws::ws_handler))
        .route("/healthz", get(healthz_handler))
        .route("/readyz", get(readyz_handler))
        .route("/info", get(info_handler))
        .with_state(state)
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
