use std::{net::SocketAddr, time::Instant};

use axum::{
    extract::{
        ConnectInfo, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use serde_json::{Value, json};
use tokio::time::timeout;
use tracing::{debug, error, warn};

use crate::{
    application::state::{ConnectedClient, SharedState, sanitize_scopes},
    protocol::{
        ConnectParams, ERROR_INVALID_REQUEST, ErrorShape, GatewayPolicy, HelloFeatures, HelloOk,
        HelloServer, PROTOCOL_VERSION, parse_request_frame, response_error, response_ok,
    },
    rpc::{SessionContext, dispatcher::dispatch_request, policy::default_operator_scopes},
    security::auth::{auth_failure_error, authorize},
    storage::now_unix_ms,
};

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    ws.max_message_size(state.config().max_payload_bytes)
        .on_upgrade(move |socket| handle_socket(socket, state, remote_addr))
}

async fn handle_socket(mut socket: WebSocket, state: SharedState, remote_addr: SocketAddr) {
    let remote_ip = Some(remote_addr.ip().to_string());

    let session = match perform_handshake(&mut socket, &state, remote_ip).await {
        Ok(context) => context,
        Err(()) => {
            debug!("handshake failed remote={remote_addr}");
            return;
        }
    };

    while let Some(next) = socket.recv().await {
        let message = match next {
            Ok(message) => message,
            Err(error) => {
                warn!("websocket receive failed conn={}: {error}", session.conn_id);
                break;
            }
        };

        let text = match message_to_text(message, state.config().max_payload_bytes) {
            Ok(Some(text)) => text,
            Ok(None) => continue,
            Err(error_shape) => {
                let response = response_error("invalid", error_shape);
                if send_response(&mut socket, response).await.is_err() {
                    break;
                }
                break;
            }
        };

        let request = match parse_request_frame(&text) {
            Ok(frame) => frame,
            Err(error_shape) => {
                let request_id = extract_frame_id(&text).unwrap_or_else(|| "invalid".to_owned());
                let response = response_error(request_id, error_shape);
                if send_response(&mut socket, response).await.is_err() {
                    break;
                }
                continue;
            }
        };

        let response = dispatch_request(&state, &session, &request).await;
        if send_response(&mut socket, response).await.is_err() {
            break;
        }
    }

    if let Err(error) = state.unregister_client(&session.conn_id).await {
        warn!(
            "failed to unregister client conn={}: {error}",
            session.conn_id
        );
    }

    debug!(
        "connection closed conn={} remote={remote_addr}",
        session.conn_id
    );
}

async fn perform_handshake(
    socket: &mut WebSocket,
    state: &SharedState,
    remote_ip: Option<String>,
) -> Result<SessionContext, ()> {
    let text = match timeout(
        state.config().handshake_timeout,
        recv_next_text(socket, state),
    )
    .await
    {
        Ok(Ok(text)) => text,
        Ok(Err(error_shape)) => {
            let response = response_error("connect", error_shape);
            let _ = send_response(socket, response).await;
            return Err(());
        }
        Err(_) => {
            let response = response_error(
                "connect",
                ErrorShape::new(ERROR_INVALID_REQUEST, "handshake timeout"),
            );
            let _ = send_response(socket, response).await;
            return Err(());
        }
    };

    let request = match parse_request_frame(&text) {
        Ok(frame) => frame,
        Err(error_shape) => {
            let request_id = extract_frame_id(&text).unwrap_or_else(|| "connect".to_owned());
            let response = response_error(request_id, error_shape);
            let _ = send_response(socket, response).await;
            return Err(());
        }
    };

    if request.method != "connect" {
        let response = response_error(
            request.id,
            ErrorShape::new(
                ERROR_INVALID_REQUEST,
                "invalid handshake: first request must be connect",
            ),
        );
        let _ = send_response(socket, response).await;
        return Err(());
    }

    let connect_params = match parse_connect_params(request.params) {
        Ok(params) => params,
        Err(error_shape) => {
            let response = response_error(request.id, error_shape);
            let _ = send_response(socket, response).await;
            return Err(());
        }
    };

    if connect_params.max_protocol < PROTOCOL_VERSION
        || connect_params.min_protocol > PROTOCOL_VERSION
    {
        let response = response_error(
            request.id,
            ErrorShape::new(ERROR_INVALID_REQUEST, "protocol mismatch")
                .with_details(json!({ "expectedProtocol": PROTOCOL_VERSION })),
        );
        let _ = send_response(socket, response).await;
        return Err(());
    }

    let role = connect_params
        .role
        .clone()
        .unwrap_or_else(|| "operator".to_owned());
    if role != "operator" && role != "node" {
        let response = response_error(
            request.id,
            ErrorShape::new(ERROR_INVALID_REQUEST, "invalid role"),
        );
        let _ = send_response(socket, response).await;
        return Err(());
    }

    let auth_key = auth_key(&remote_ip, &connect_params.client.id);
    let limiter = state.auth_rate_limiter();
    let decision = limiter.check(&auth_key).await;
    if !decision.allowed {
        let response = response_error(
            request.id,
            ErrorShape::new(
                crate::protocol::ERROR_UNAVAILABLE,
                "unauthorized: too many failed attempts",
            )
            .with_retry(decision.retry_after_ms),
        );
        let _ = send_response(socket, response).await;
        return Err(());
    }

    if let Err(reason) = authorize(&state.config().auth_mode, connect_params.auth.as_ref()) {
        let record = limiter.record_failure(&auth_key).await;
        let mut shape = auth_failure_error(reason);
        if !record.allowed || record.retry_after_ms > 0 {
            shape = shape.with_retry(record.retry_after_ms);
        }

        let response = response_error(request.id, shape);
        let _ = send_response(socket, response).await;
        return Err(());
    }

    limiter.reset(&auth_key).await;

    let conn_id = uuid::Uuid::new_v4().to_string();
    let mut scopes = sanitize_scopes(&connect_params.scopes);
    if role == "operator" && scopes.is_empty() {
        scopes = default_operator_scopes();
    }
    let connected_at = Instant::now();
    let connected_at_ms = now_unix_ms();

    let registered_client = ConnectedClient {
        conn_id: conn_id.clone(),
        client_id: connect_params.client.id.clone(),
        display_name: connect_params.client.display_name.clone(),
        client_version: connect_params.client.version.clone(),
        platform: connect_params.client.platform.clone(),
        device_family: connect_params.client.device_family.clone(),
        model_identifier: connect_params.client.model_identifier.clone(),
        mode: connect_params.client.mode.clone(),
        role: role.clone(),
        scopes: scopes.clone(),
        instance_id: connect_params.client.instance_id.clone(),
        remote_ip,
        connected_at,
        connected_at_ms,
    };

    if let Err(error) = state.register_client(registered_client).await {
        let response = response_error(
            request.id,
            ErrorShape::new(
                crate::protocol::ERROR_UNAVAILABLE,
                format!("failed to register connection: {error}"),
            ),
        );
        let _ = send_response(socket, response).await;
        return Err(());
    }

    let snapshot = match state.snapshot().await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = state.unregister_client(&conn_id).await;
            let response = response_error(
                request.id,
                ErrorShape::new(
                    crate::protocol::ERROR_UNAVAILABLE,
                    format!("failed to build snapshot: {error}"),
                ),
            );
            let _ = send_response(socket, response).await;
            return Err(());
        }
    };

    let hello = HelloOk {
        frame_type: "hello-ok",
        protocol: PROTOCOL_VERSION,
        server: HelloServer {
            version: state.config().runtime_version.clone(),
            conn_id: conn_id.clone(),
        },
        features: HelloFeatures {
            methods: state.methods(),
            events: state.events(),
        },
        snapshot,
        canvas_host_url: None,
        auth: None,
        policy: GatewayPolicy {
            max_payload: state.config().max_payload_bytes,
            max_buffered_bytes: state.config().max_buffered_bytes,
            tick_interval_ms: state.config().tick_interval_ms,
        },
    };

    let payload = match serde_json::to_value(hello) {
        Ok(value) => value,
        Err(error) => {
            let _ = state.unregister_client(&conn_id).await;
            let response = response_error(
                request.id,
                ErrorShape::new(
                    crate::protocol::ERROR_UNAVAILABLE,
                    format!("failed to serialize hello payload: {error}"),
                ),
            );
            let _ = send_response(socket, response).await;
            return Err(());
        }
    };

    let response = response_ok(request.id, payload);
    if send_response(socket, response).await.is_err() {
        let _ = state.unregister_client(&conn_id).await;
        return Err(());
    }

    debug!("handshake ok conn={conn_id} role={role}");
    Ok(SessionContext {
        conn_id,
        role,
        scopes,
        client_id: connect_params.client.id,
        client_mode: connect_params.client.mode,
    })
}

async fn recv_next_text(socket: &mut WebSocket, state: &SharedState) -> Result<String, ErrorShape> {
    loop {
        let next = socket.recv().await.ok_or_else(|| {
            ErrorShape::new(ERROR_INVALID_REQUEST, "connection closed before handshake")
        })?;

        let message = next.map_err(|error| {
            ErrorShape::new(
                crate::protocol::ERROR_UNAVAILABLE,
                format!("websocket read failed: {error}"),
            )
        })?;

        match message_to_text(message, state.config().max_payload_bytes)? {
            Some(text) => return Ok(text),
            None => continue,
        }
    }
}

fn parse_connect_params(params: Option<Value>) -> Result<ConnectParams, ErrorShape> {
    let raw = params.unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    serde_json::from_value::<ConnectParams>(raw).map_err(|error| {
        ErrorShape::new(
            ERROR_INVALID_REQUEST,
            format!("invalid connect params: {error}"),
        )
    })
}

fn auth_key(remote_ip: &Option<String>, client_id: &str) -> String {
    let ip = remote_ip.as_deref().unwrap_or("unknown");
    format!("{ip}:{client_id}")
}

async fn send_response(
    socket: &mut WebSocket,
    response: crate::protocol::ResponseFrame,
) -> Result<(), ()> {
    let text = match serde_json::to_string(&response) {
        Ok(value) => value,
        Err(error) => {
            error!("failed to serialize websocket response: {error}");
            return Err(());
        }
    };

    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|error| {
            warn!("failed to send websocket response: {error}");
        })
}

fn message_to_text(
    message: Message,
    max_payload_bytes: usize,
) -> Result<Option<String>, ErrorShape> {
    match message {
        Message::Text(text) => {
            if text.len() > max_payload_bytes {
                return Err(ErrorShape::new(
                    ERROR_INVALID_REQUEST,
                    format!(
                        "payload exceeds maxPayload ({} > {})",
                        text.len(),
                        max_payload_bytes
                    ),
                ));
            }
            Ok(Some(text.to_string()))
        }
        Message::Binary(bytes) => {
            if bytes.len() > max_payload_bytes {
                return Err(ErrorShape::new(
                    ERROR_INVALID_REQUEST,
                    format!(
                        "payload exceeds maxPayload ({} > {})",
                        bytes.len(),
                        max_payload_bytes
                    ),
                ));
            }
            let text = String::from_utf8(bytes.to_vec()).map_err(|_| {
                ErrorShape::new(
                    ERROR_INVALID_REQUEST,
                    "binary websocket frames must contain UTF-8",
                )
            })?;
            Ok(Some(text))
        }
        Message::Ping(_) | Message::Pong(_) => Ok(None),
        Message::Close(_) => Ok(None),
    }
}

fn extract_frame_id(text: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(text).ok()?;
    let id = value.get("id")?.as_str()?;
    let trimmed = id.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::extract_frame_id;

    #[test]
    fn frame_id_parser_handles_missing_values() {
        assert_eq!(extract_frame_id("{}"), None);
        assert_eq!(extract_frame_id(r#"{"id":""}"#), None);
        assert_eq!(extract_frame_id(r#"{"id":"abc"}"#), Some("abc".to_owned()));
    }
}
