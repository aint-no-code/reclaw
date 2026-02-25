use futures_util::SinkExt;
use reclaw_core::{application::config::AuthMode, protocol::PROTOCOL_VERSION};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use super::support::{connect_frame, connect_gateway, recv_json, rpc_req, spawn_server_with};

async fn assert_session_has_history(server_addr: std::net::SocketAddr, session_key: &str) {
    let mut ws = connect_gateway(server_addr).await;
    ws.send(Message::Text(
        connect_frame(None, 1, PROTOCOL_VERSION, "operator", "reclaw-test", &[])
            .to_string()
            .into(),
    ))
    .await
    .expect("connect frame should send");
    let _ = recv_json(&mut ws).await;

    let history = rpc_req(
        &mut ws,
        "history-1",
        "chat.history",
        Some(json!({
            "sessionKey": session_key,
            "limit": 20
        })),
    )
    .await;
    assert_eq!(history["ok"], true);
    assert!(
        history["payload"]["messages"]
            .as_array()
            .is_some_and(|messages| messages.len() >= 2)
    );
}

#[tokio::test]
async fn hooks_routes_are_disabled_by_default() {
    let server = spawn_server_with(AuthMode::None, |_| {}).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/hooks/agent", server.addr))
        .bearer_auth("hooks-token")
        .json(&json!({
            "message": "hello",
        }))
        .send()
        .await
        .expect("hooks request should return");

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    server.stop().await;
}

#[tokio::test]
async fn hooks_agent_requires_valid_token() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.hooks_enabled = true;
        config.hooks_token = Some("hooks-token".to_owned());
    })
    .await;

    let client = reqwest::Client::new();
    let unauthorized = client
        .post(format!("http://{}/hooks/agent", server.addr))
        .json(&json!({
            "message": "hello",
        }))
        .send()
        .await
        .expect("hooks request should return");
    assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);

    let authorized = client
        .post(format!("http://{}/hooks/agent", server.addr))
        .bearer_auth("hooks-token")
        .json(&json!({
            "message": "hello",
        }))
        .send()
        .await
        .expect("hooks request should return");
    assert_eq!(authorized.status(), reqwest::StatusCode::ACCEPTED);

    server.stop().await;
}

#[tokio::test]
async fn hooks_reject_token_query_parameter() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.hooks_enabled = true;
        config.hooks_token = Some("hooks-token".to_owned());
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}/hooks/agent?token=hooks-token",
            server.addr
        ))
        .json(&json!({
            "message": "hello",
        }))
        .send()
        .await
        .expect("hooks request should return");

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "INVALID_REQUEST");

    server.stop().await;
}

#[tokio::test]
async fn hooks_agent_dispatches_with_default_session_key() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.hooks_enabled = true;
        config.hooks_token = Some("hooks-token".to_owned());
        config.hooks_default_session_key = Some("hook:integration".to_owned());
        config.hooks_default_agent_id = "ops".to_owned();
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/hooks/agent", server.addr))
        .bearer_auth("hooks-token")
        .json(&json!({
            "message": "hook hello",
        }))
        .send()
        .await
        .expect("hooks request should return");

    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["sessionKey"], "hook:integration");
    assert_eq!(payload["agentId"], "ops");
    assert!(
        payload["runId"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );

    assert_session_has_history(server.addr, "hook:integration").await;
    server.stop().await;
}

#[tokio::test]
async fn hooks_agent_session_key_policy_is_enforced() {
    let server_a = spawn_server_with(AuthMode::None, |config| {
        config.hooks_enabled = true;
        config.hooks_token = Some("hooks-token".to_owned());
    })
    .await;

    let client = reqwest::Client::new();
    let blocked = client
        .post(format!("http://{}/hooks/agent", server_a.addr))
        .bearer_auth("hooks-token")
        .json(&json!({
            "message": "hook hello",
            "sessionKey": "hook:custom",
        }))
        .send()
        .await
        .expect("hooks request should return");
    assert_eq!(blocked.status(), reqwest::StatusCode::BAD_REQUEST);
    let blocked_payload: Value = blocked.json().await.expect("response should be json");
    assert_eq!(blocked_payload["ok"], false);
    assert_eq!(blocked_payload["error"]["code"], "INVALID_REQUEST");

    server_a.stop().await;

    let server_b = spawn_server_with(AuthMode::None, |config| {
        config.hooks_enabled = true;
        config.hooks_token = Some("hooks-token".to_owned());
        config.hooks_allow_request_session_key = true;
    })
    .await;

    let allowed = client
        .post(format!("http://{}/hooks/agent", server_b.addr))
        .bearer_auth("hooks-token")
        .json(&json!({
            "message": "hook hello",
            "sessionKey": "hook:custom",
        }))
        .send()
        .await
        .expect("hooks request should return");
    assert_eq!(allowed.status(), reqwest::StatusCode::ACCEPTED);
    let allowed_payload: Value = allowed.json().await.expect("response should be json");
    assert_eq!(allowed_payload["sessionKey"], "hook:custom");

    assert_session_has_history(server_b.addr, "hook:custom").await;
    server_b.stop().await;
}

#[tokio::test]
async fn hooks_wake_updates_last_heartbeat_for_now_mode() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.hooks_enabled = true;
        config.hooks_token = Some("hooks-token".to_owned());
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/hooks/wake", server.addr))
        .bearer_auth("hooks-token")
        .json(&json!({
            "text": "wake now",
        }))
        .send()
        .await
        .expect("hooks request should return");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["mode"], "now");

    let mut ws = connect_gateway(server.addr).await;
    ws.send(Message::Text(
        connect_frame(None, 1, PROTOCOL_VERSION, "operator", "reclaw-test", &[])
            .to_string()
            .into(),
    ))
    .await
    .expect("connect frame should send");
    let _ = recv_json(&mut ws).await;

    let heartbeat = rpc_req(&mut ws, "wake-1", "last-heartbeat", Some(json!({}))).await;
    assert_eq!(heartbeat["ok"], true);
    assert_eq!(heartbeat["payload"]["reason"], "hook:wake");

    server.stop().await;
}
