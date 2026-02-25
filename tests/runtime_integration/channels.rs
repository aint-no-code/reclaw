use std::net::Ipv4Addr;

use axum::{Json, Router, routing::post};
use futures_util::SinkExt;
use reclaw_core::application::config::AuthMode;
use reclaw_core::application::state::SharedState;
use reclaw_core::interfaces::webhooks::{
    ChannelWebhookAdapter, ChannelWebhookRegistry, WebhookFuture,
};
use reclaw_core::protocol::PROTOCOL_VERSION;
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot},
    time::timeout,
};
use tokio_tungstenite::tungstenite::Message;

use super::support::{
    connect_frame, connect_gateway, recv_json, rpc_req, spawn_server_with,
    spawn_server_with_webhooks,
};

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
async fn channels_inbound_requires_bearer_token_when_configured() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.channels_inbound_token = Some("bridge-token".to_owned());
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/channels/inbound", server.addr))
        .json(&json!({
            "channel": "telegram",
            "conversationId": "chat-1",
            "text": "hello",
        }))
        .send()
        .await
        .expect("inbound request should return");

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], false);

    server.stop().await;
}

#[tokio::test]
async fn channels_inbound_routes_message_to_chat_session() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.channels_inbound_token = Some("bridge-token".to_owned());
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/channels/inbound", server.addr))
        .bearer_auth("bridge-token")
        .json(&json!({
            "channel": "telegram",
            "conversationId": "12345",
            "text": "hello from channel",
            "messageId": "m1"
        }))
        .send()
        .await
        .expect("inbound request should return");

    assert!(response.status().is_success());
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], true);

    let mut ws = connect_gateway(server.addr).await;
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
        "inbound-1",
        "chat.history",
        Some(json!({
            "sessionKey": "agent:main:telegram:chat:12345",
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

    server.stop().await;
}

#[tokio::test]
async fn channel_specific_inbound_route_uses_path_channel() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.channels_inbound_token = Some("bridge-token".to_owned());
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/channels/slack/inbound", server.addr))
        .bearer_auth("bridge-token")
        .json(&json!({
            "conversationId": "team-chat-7",
            "text": "hello from path route",
            "messageId": "m2"
        }))
        .send()
        .await
        .expect("inbound request should return");

    assert!(response.status().is_success());
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["sessionKey"], "agent:main:slack:chat:team-chat-7");

    let mut ws = connect_gateway(server.addr).await;
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
        "inbound-2",
        "chat.history",
        Some(json!({
            "sessionKey": "agent:main:slack:chat:team-chat-7",
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

    server.stop().await;
}

#[tokio::test]
async fn telegram_webhook_rejects_invalid_secret() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.telegram_webhook_secret = Some("secret-123".to_owned());
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/channels/telegram/webhook", server.addr))
        .json(&json!({
            "update_id": 100,
            "message": {
                "message_id": 1,
                "chat": { "id": 42 },
                "text": "hello"
            }
        }))
        .send()
        .await
        .expect("telegram webhook should return");

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], false);

    server.stop().await;
}

#[tokio::test]
async fn telegram_webhook_ingests_updates_and_deduplicates() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.telegram_webhook_secret = Some("secret-123".to_owned());
    })
    .await;

    let client = reqwest::Client::new();
    let url = format!("http://{}/channels/telegram/webhook", server.addr);

    let first = client
        .post(&url)
        .header("x-telegram-bot-api-secret-token", "secret-123")
        .json(&json!({
            "update_id": 101,
            "message": {
                "message_id": 11,
                "chat": { "id": 555 },
                "from": { "id": 333 },
                "text": "hello from telegram"
            }
        }))
        .send()
        .await
        .expect("telegram webhook should return");
    assert!(first.status().is_success());
    let first_payload: Value = first.json().await.expect("response should be json");
    assert_eq!(first_payload["ok"], true);
    assert_eq!(first_payload["accepted"], true);

    let duplicate = client
        .post(&url)
        .header("x-telegram-bot-api-secret-token", "secret-123")
        .json(&json!({
            "update_id": 101,
            "message": {
                "message_id": 11,
                "chat": { "id": 555 },
                "text": "hello from telegram"
            }
        }))
        .send()
        .await
        .expect("telegram webhook should return");
    assert!(duplicate.status().is_success());
    let duplicate_payload: Value = duplicate.json().await.expect("response should be json");
    assert_eq!(duplicate_payload["ok"], true);
    assert_eq!(duplicate_payload["duplicate"], true);

    let mut ws = connect_gateway(server.addr).await;
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
        "telegram-1",
        "chat.history",
        Some(json!({
            "sessionKey": "agent:main:telegram:chat:555",
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

    server.stop().await;
}

#[tokio::test]
async fn telegram_webhook_can_send_outbound_reply() {
    let mock_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("mock listener should bind");
    let mock_addr = mock_listener
        .local_addr()
        .expect("mock listener should expose local addr");
    let (mock_shutdown_tx, mock_shutdown_rx) = oneshot::channel::<()>();
    let (body_tx, mut body_rx) = mpsc::unbounded_channel::<Value>();

    let app = Router::new().route(
        "/bottest-token/sendMessage",
        post({
            let body_tx = body_tx.clone();
            move |Json(body): Json<Value>| {
                let body_tx = body_tx.clone();
                async move {
                    let _ = body_tx.send(body);
                    Json(json!({
                        "ok": true,
                        "result": {"message_id": 9}
                    }))
                }
            }
        }),
    );
    let mock_join = tokio::spawn(async move {
        let _ = axum::serve(mock_listener, app)
            .with_graceful_shutdown(async {
                let _ = mock_shutdown_rx.await;
            })
            .await;
    });

    let server = spawn_server_with(AuthMode::None, |config| {
        config.telegram_webhook_secret = Some("secret-123".to_owned());
        config.telegram_bot_token = Some("test-token".to_owned());
        config.telegram_api_base_url = format!("http://{mock_addr}");
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/channels/telegram/webhook", server.addr))
        .header("x-telegram-bot-api-secret-token", "secret-123")
        .json(&json!({
            "update_id": 202,
            "message": {
                "message_id": 2,
                "chat": { "id": 777 },
                "text": "reply please"
            }
        }))
        .send()
        .await
        .expect("telegram webhook should return");

    assert!(response.status().is_success());
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["outboundSent"], true);

    let outbound = timeout(std::time::Duration::from_secs(2), body_rx.recv())
        .await
        .expect("telegram outbound request should arrive")
        .expect("outbound payload should exist");

    assert_eq!(outbound["chat_id"], 777);
    assert!(
        outbound["text"]
            .as_str()
            .is_some_and(|text| text.contains("Echo:"))
    );

    let _ = mock_shutdown_tx.send(());
    let _ = mock_join.await;
    server.stop().await;
}

#[tokio::test]
async fn channel_webhook_dispatches_to_registered_adapter() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.telegram_webhook_secret = Some("secret-123".to_owned());
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/channels/telegram/webhook", server.addr))
        .header("x-telegram-bot-api-secret-token", "secret-123")
        .json(&json!({
            "update_id": 303,
            "message": {
                "message_id": 3,
                "chat": { "id": 909 },
                "text": "via adapter registry"
            }
        }))
        .send()
        .await
        .expect("channel webhook should return");

    assert!(response.status().is_success());
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["accepted"], true);
    assert_eq!(payload["sessionKey"], "agent:main:telegram:chat:909");

    server.stop().await;
}

#[tokio::test]
async fn channel_webhook_rejects_unknown_adapter() {
    let server = spawn_server_with(AuthMode::None, |_| {}).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/channels/unknown/webhook", server.addr))
        .json(&json!({
            "foo": "bar"
        }))
        .send()
        .await
        .expect("channel webhook should return");

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "NOT_FOUND");

    server.stop().await;
}

#[tokio::test]
async fn slack_webhook_requires_configured_token() {
    let server = spawn_server_with(AuthMode::None, |_| {}).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/channels/slack/webhook", server.addr))
        .json(&json!({
            "type": "event_callback",
            "event_id": "Ev-unauth",
            "event": {
                "type": "message",
                "channel": "C-1",
                "text": "hello"
            }
        }))
        .send()
        .await
        .expect("slack webhook should return");

    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "UNAVAILABLE");

    server.stop().await;
}

fn fake_webhook_dispatch<'a>(
    _state: &'a SharedState,
    _headers: &'a axum::http::HeaderMap,
    payload: Value,
) -> WebhookFuture<'a> {
    Box::pin(async move {
        (
            axum::http::StatusCode::OK,
            Json(json!({
                "ok": true,
                "channel": "fakechat",
                "payload": payload,
            })),
        )
    })
}

#[tokio::test]
async fn channel_webhook_supports_runtime_injected_registry_adapters() {
    let mut registry = ChannelWebhookRegistry::default();
    registry.register(ChannelWebhookAdapter {
        channel: "fakechat",
        dispatch: fake_webhook_dispatch,
    });

    let server = spawn_server_with_webhooks(AuthMode::None, |_| {}, registry).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/channels/fakechat/webhook", server.addr))
        .json(&json!({
            "hello": "world"
        }))
        .send()
        .await
        .expect("channel webhook should return");

    assert!(response.status().is_success());
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["channel"], "fakechat");
    assert_eq!(payload["payload"]["hello"], "world");

    server.stop().await;
}

#[tokio::test]
async fn slack_webhook_ingests_event_message() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.slack_webhook_token = Some("slack-token".to_owned());
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/channels/slack/webhook", server.addr))
        .bearer_auth("slack-token")
        .json(&json!({
            "type": "event_callback",
            "event_id": "Ev-1",
            "event": {
                "type": "message",
                "channel": "C123",
                "user": "U123",
                "text": "hello from slack",
                "ts": "111.222"
            }
        }))
        .send()
        .await
        .expect("slack webhook should return");

    assert!(response.status().is_success());
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["accepted"], true);

    let session_key = payload["sessionKey"]
        .as_str()
        .expect("session key should exist")
        .to_owned();
    assert_session_has_history(server.addr, &session_key).await;

    server.stop().await;
}

#[tokio::test]
async fn discord_webhook_ingests_message_payload() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.discord_webhook_token = Some("discord-token".to_owned());
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/channels/discord/webhook", server.addr))
        .bearer_auth("discord-token")
        .json(&json!({
            "id": "msg-1",
            "channel_id": "chan-1",
            "content": "hello from discord",
            "author": { "id": "user-1" }
        }))
        .send()
        .await
        .expect("discord webhook should return");

    assert!(response.status().is_success());
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["accepted"], true);

    let session_key = payload["sessionKey"]
        .as_str()
        .expect("session key should exist")
        .to_owned();
    assert_session_has_history(server.addr, &session_key).await;

    server.stop().await;
}

#[tokio::test]
async fn signal_webhook_ingests_envelope_payload() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.signal_webhook_token = Some("signal-token".to_owned());
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/channels/signal/webhook", server.addr))
        .bearer_auth("signal-token")
        .json(&json!({
            "envelope": {
                "sourceNumber": "+123456789",
                "timestamp": 1700000000,
                "dataMessage": {
                    "message": "hello from signal"
                }
            }
        }))
        .send()
        .await
        .expect("signal webhook should return");

    assert!(response.status().is_success());
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["accepted"], true);

    let session_key = payload["sessionKey"]
        .as_str()
        .expect("session key should exist")
        .to_owned();
    assert_session_has_history(server.addr, &session_key).await;

    server.stop().await;
}

#[tokio::test]
async fn whatsapp_webhook_ingests_cloud_payload() {
    let server = spawn_server_with(AuthMode::None, |config| {
        config.whatsapp_webhook_token = Some("whatsapp-token".to_owned());
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/channels/whatsapp/webhook", server.addr))
        .bearer_auth("whatsapp-token")
        .json(&json!({
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [{
                            "id": "wamid.1",
                            "from": "15551234567",
                            "text": { "body": "hello from whatsapp" }
                        }]
                    }
                }]
            }]
        }))
        .send()
        .await
        .expect("whatsapp webhook should return");

    assert!(response.status().is_success());
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["accepted"], true);

    let session_key = payload["sessionKey"]
        .as_str()
        .expect("session key should exist")
        .to_owned();
    assert_session_has_history(server.addr, &session_key).await;

    server.stop().await;
}
