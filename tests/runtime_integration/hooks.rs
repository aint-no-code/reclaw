use futures_util::SinkExt;
use reclaw_core::{
    application::config::{AuthMode, HookMappingAction, HookMappingConfig},
    protocol::PROTOCOL_VERSION,
};
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

async fn session_history_texts(
    server_addr: std::net::SocketAddr,
    session_key: &str,
) -> Vec<String> {
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
        "history-texts-1",
        "chat.history",
        Some(json!({
            "sessionKey": session_key,
            "limit": 20
        })),
    )
    .await;
    history["payload"]["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| entry.get("text").and_then(Value::as_str).map(str::to_owned))
        .collect()
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

#[tokio::test]
async fn hooks_mapping_dispatches_agent_action() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.hooks_enabled = true;
        config.hooks_token = Some("hooks-token".to_owned());
        config.hooks_mappings = vec![HookMappingConfig {
            id: Some("github".to_owned()),
            path: "github/push".to_owned(),
            action: HookMappingAction::Agent,
            match_source: None,
            wake_mode: None,
            text: None,
            text_template: None,
            message: Some("mapped github event".to_owned()),
            message_template: None,
            name: Some("GitHub".to_owned()),
            agent_id: Some("mapped-agent".to_owned()),
            session_key: Some("hook:mapped".to_owned()),
        }];
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/hooks/github/push", server.addr))
        .bearer_auth("hooks-token")
        .json(&json!({}))
        .send()
        .await
        .expect("hooks request should return");

    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["sessionKey"], "hook:mapped");
    assert_eq!(payload["agentId"], "mapped-agent");
    assert!(
        payload["runId"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );

    assert_session_has_history(server.addr, "hook:mapped").await;
    server.stop().await;
}

#[tokio::test]
async fn hooks_mapping_dispatches_wake_action() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.hooks_enabled = true;
        config.hooks_token = Some("hooks-token".to_owned());
        config.hooks_mappings = vec![HookMappingConfig {
            id: Some("watchdog".to_owned()),
            path: "/watchdog/ping/".to_owned(),
            action: HookMappingAction::Wake,
            match_source: None,
            wake_mode: Some("next-heartbeat".to_owned()),
            text: Some("watchdog ping".to_owned()),
            text_template: None,
            message: None,
            message_template: None,
            name: None,
            agent_id: None,
            session_key: None,
        }];
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/hooks/watchdog/ping", server.addr))
        .bearer_auth("hooks-token")
        .json(&json!({}))
        .send()
        .await
        .expect("hooks request should return");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["mode"], "next-heartbeat");

    server.stop().await;
}

#[tokio::test]
async fn hooks_mapping_honors_match_source_filter() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.hooks_enabled = true;
        config.hooks_token = Some("hooks-token".to_owned());
        config.hooks_mappings = vec![HookMappingConfig {
            id: Some("source-filter".to_owned()),
            path: "events/source".to_owned(),
            action: HookMappingAction::Agent,
            match_source: Some("github".to_owned()),
            wake_mode: None,
            text: None,
            text_template: None,
            message: Some("source matched".to_owned()),
            message_template: None,
            name: None,
            agent_id: None,
            session_key: Some("hook:source-filter".to_owned()),
        }];
    })
    .await;

    let client = reqwest::Client::new();
    let rejected = client
        .post(format!("http://{}/hooks/events/source", server.addr))
        .bearer_auth("hooks-token")
        .json(&json!({
            "source": "slack",
        }))
        .send()
        .await
        .expect("hooks request should return");
    assert_eq!(rejected.status(), reqwest::StatusCode::NOT_FOUND);

    let accepted = client
        .post(format!("http://{}/hooks/events/source", server.addr))
        .bearer_auth("hooks-token")
        .json(&json!({
            "source": "github",
        }))
        .send()
        .await
        .expect("hooks request should return");
    assert_eq!(accepted.status(), reqwest::StatusCode::ACCEPTED);

    assert_session_has_history(server.addr, "hook:source-filter").await;
    server.stop().await;
}

#[tokio::test]
async fn hooks_mapping_renders_message_template_from_payload() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.hooks_enabled = true;
        config.hooks_token = Some("hooks-token".to_owned());
        config.hooks_mappings = vec![HookMappingConfig {
            id: Some("template".to_owned()),
            path: "github/template".to_owned(),
            action: HookMappingAction::Agent,
            match_source: None,
            wake_mode: None,
            text: None,
            text_template: None,
            message: None,
            message_template: Some(
                "repo={{repo}} actor={{actor.name}} first={{commits[0].id}}".to_owned(),
            ),
            name: None,
            agent_id: None,
            session_key: Some("hook:template".to_owned()),
        }];
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/hooks/github/template", server.addr))
        .bearer_auth("hooks-token")
        .json(&json!({
            "repo": "reclaw",
            "actor": { "name": "jd" },
            "commits": [{ "id": "c1" }]
        }))
        .send()
        .await
        .expect("hooks request should return");

    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);

    let history_texts = session_history_texts(server.addr, "hook:template").await;
    assert!(
        history_texts
            .iter()
            .any(|text| text.contains("repo=reclaw actor=jd first=c1"))
    );

    server.stop().await;
}

#[tokio::test]
async fn hooks_mapping_renders_header_query_and_path_context() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.hooks_enabled = true;
        config.hooks_token = Some("hooks-token".to_owned());
        config.hooks_mappings = vec![HookMappingConfig {
            id: Some("context".to_owned()),
            path: "context/run".to_owned(),
            action: HookMappingAction::Agent,
            match_source: None,
            wake_mode: None,
            text: None,
            text_template: None,
            message: None,
            message_template: Some(
                "ua={{headers.user-agent}} kind={{query.kind}} path={{path}}".to_owned(),
            ),
            name: None,
            agent_id: None,
            session_key: Some("hook:context".to_owned()),
        }];
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "http://{}/hooks/context/run?kind=push",
            server.addr
        ))
        .bearer_auth("hooks-token")
        .header("User-Agent", "reclaw-test/1")
        .json(&json!({}))
        .send()
        .await
        .expect("hooks request should return");

    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);

    let history_texts = session_history_texts(server.addr, "hook:context").await;
    assert!(
        history_texts
            .iter()
            .any(|text| text.contains("ua=reclaw-test/1 kind=push path=context/run"))
    );

    server.stop().await;
}
