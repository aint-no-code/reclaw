use std::net::Ipv4Addr;

use axum::{Json, Router, routing::post};
use futures_util::SinkExt;
use reclaw_core::application::config::AuthMode;
use reclaw_core::protocol::PROTOCOL_VERSION;
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot},
    time::timeout,
};
use tokio_tungstenite::tungstenite::Message;

use super::support::{connect_frame, connect_gateway, recv_json, rpc_req, spawn_server_with};

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
