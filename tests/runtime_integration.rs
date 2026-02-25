use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use futures_util::{SinkExt, StreamExt};
use reclaw_server::application::{
    config::{AuthMode, RuntimeConfig},
    startup,
};
use reclaw_server::protocol::PROTOCOL_VERSION;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

struct ServerHandle {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
    _temp_dir: TempDir,
}

impl ServerHandle {
    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = self.join.await;
    }
}

async fn spawn_server(auth_mode: AuthMode) -> ServerHandle {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should expose local addr");

    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let db_path = temp_dir.path().join("reclaw.db");

    let mut config = RuntimeConfig::for_test(IpAddr::V4(Ipv4Addr::LOCALHOST), addr.port(), db_path);
    config.auth_mode = auth_mode;

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let join = tokio::spawn(async move {
        let _ = startup::run_with_listener(listener, config, async {
            let _ = shutdown_rx.await;
        })
        .await;
    });

    ServerHandle {
        addr,
        shutdown: Some(shutdown_tx),
        join,
        _temp_dir: temp_dir,
    }
}

async fn connect_gateway(
    addr: SocketAddr,
) -> WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>> {
    let (socket, _) = connect_async(format!("ws://{addr}/"))
        .await
        .expect("websocket should connect");
    socket
}

fn connect_frame(auth_token: Option<&str>, min_protocol: u32, max_protocol: u32) -> Value {
    json!({
        "type": "req",
        "id": "connect-1",
        "method": "connect",
        "params": {
            "minProtocol": min_protocol,
            "maxProtocol": max_protocol,
            "client": {
                "id": "reclaw-test",
                "displayName": "Reclaw Test",
                "version": "0.0.1",
                "platform": "test",
                "mode": "cli"
            },
            "role": "operator",
            "auth": {
                "token": auth_token
            }
        }
    })
}

async fn recv_json(ws: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>) -> Value {
    while let Some(next) = ws.next().await {
        let message = next.expect("websocket stream should remain valid");
        match message {
            Message::Text(text) => {
                return serde_json::from_str(text.as_ref()).expect("json payload expected");
            }
            Message::Binary(bytes) => {
                return serde_json::from_slice(bytes.as_ref()).expect("json payload expected");
            }
            Message::Ping(payload) => {
                ws.send(Message::Pong(payload))
                    .await
                    .expect("pong should send");
            }
            Message::Pong(_) => {}
            Message::Close(_) => panic!("websocket closed before payload"),
            Message::Frame(_) => {}
        }
    }

    panic!("websocket ended unexpectedly");
}

async fn rpc_req(
    ws: &mut WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    id: &str,
    method: &str,
    params: Option<Value>,
) -> Value {
    let mut request = json!({
        "type": "req",
        "id": id,
        "method": method,
    });

    if let Some(params) = params {
        request["params"] = params;
    }

    ws.send(Message::Text(request.to_string().into()))
        .await
        .expect("request should send");

    recv_json(ws).await
}

#[tokio::test]
async fn handshake_and_health_round_trip() {
    let server = spawn_server(AuthMode::None).await;
    let mut ws = connect_gateway(server.addr).await;

    ws.send(Message::Text(
        connect_frame(None, 1, PROTOCOL_VERSION).to_string().into(),
    ))
    .await
    .expect("connect frame should send");

    let hello = recv_json(&mut ws).await;
    assert_eq!(hello["type"], "res");
    assert_eq!(hello["id"], "connect-1");
    assert_eq!(hello["ok"], true);
    assert_eq!(hello["payload"]["type"], "hello-ok");
    assert_eq!(hello["payload"]["protocol"], PROTOCOL_VERSION);

    let health = rpc_req(&mut ws, "h-1", "health", None).await;
    assert_eq!(health["type"], "res");
    assert_eq!(health["id"], "h-1");
    assert_eq!(health["ok"], true);
    assert_eq!(health["payload"]["ok"], true);

    server.stop().await;
}

#[tokio::test]
async fn handshake_rejects_protocol_mismatch() {
    let server = spawn_server(AuthMode::None).await;
    let mut ws = connect_gateway(server.addr).await;

    ws.send(Message::Text(
        connect_frame(None, PROTOCOL_VERSION + 1, PROTOCOL_VERSION + 1)
            .to_string()
            .into(),
    ))
    .await
    .expect("connect frame should send");

    let response = recv_json(&mut ws).await;
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], "INVALID_REQUEST");
    assert_eq!(response["error"]["message"], "protocol mismatch");

    server.stop().await;
}

#[tokio::test]
async fn token_auth_rejects_missing_token() {
    let server = spawn_server(AuthMode::Token("top-secret".to_owned())).await;
    let mut ws = connect_gateway(server.addr).await;

    ws.send(Message::Text(
        connect_frame(None, 1, PROTOCOL_VERSION).to_string().into(),
    ))
    .await
    .expect("connect frame should send");

    let response = recv_json(&mut ws).await;
    assert_eq!(response["ok"], false);
    assert_eq!(
        response["error"]["message"],
        "unauthorized: missing credentials"
    );

    server.stop().await;
}

#[tokio::test]
async fn unknown_and_unimplemented_methods_have_explicit_errors() {
    let server = spawn_server(AuthMode::None).await;
    let mut ws = connect_gateway(server.addr).await;

    ws.send(Message::Text(
        connect_frame(None, 1, PROTOCOL_VERSION).to_string().into(),
    ))
    .await
    .expect("connect frame should send");
    let _ = recv_json(&mut ws).await;

    let unknown = rpc_req(&mut ws, "u-1", "unknown.method", None).await;
    assert_eq!(unknown["ok"], false);
    assert_eq!(unknown["error"]["code"], "INVALID_REQUEST");

    let unimplemented = rpc_req(&mut ws, "u-2", "wizard.start", None).await;
    assert_eq!(unimplemented["ok"], false);
    assert_eq!(unimplemented["error"]["code"], "UNAVAILABLE");

    server.stop().await;
}

#[tokio::test]
async fn method_groups_round_trip() {
    let server = spawn_server(AuthMode::None).await;
    let mut ws = connect_gateway(server.addr).await;

    ws.send(Message::Text(
        connect_frame(None, 1, PROTOCOL_VERSION).to_string().into(),
    ))
    .await
    .expect("connect frame should send");
    let _ = recv_json(&mut ws).await;

    let set_cfg = rpc_req(
        &mut ws,
        "cfg-1",
        "config.set",
        Some(json!({ "config": { "gateway": { "name": "reclaw" } } })),
    )
    .await;
    assert_eq!(set_cfg["ok"], true);

    let patch_cfg = rpc_req(
        &mut ws,
        "cfg-2",
        "config.patch",
        Some(json!({ "patch": { "gateway": { "port": 1234 } } })),
    )
    .await;
    assert_eq!(patch_cfg["ok"], true);

    let get_cfg = rpc_req(&mut ws, "cfg-3", "config.get", Some(json!({}))).await;
    assert_eq!(get_cfg["ok"], true);
    assert_eq!(get_cfg["payload"]["gateway"]["name"], "reclaw");
    assert_eq!(get_cfg["payload"]["gateway"]["port"], 1234);

    let session_patch = rpc_req(
        &mut ws,
        "sess-1",
        "sessions.patch",
        Some(json!({
            "id": "agent:main:main",
            "title": "Main",
            "tags": ["chat"],
            "metadata": {"source": "test"}
        })),
    )
    .await;
    assert_eq!(session_patch["ok"], true);

    let send = rpc_req(
        &mut ws,
        "chat-1",
        "chat.send",
        Some(json!({
            "sessionKey": "agent:main:main",
            "message": "hello",
            "idempotencyKey": "run-chat-1"
        })),
    )
    .await;
    assert_eq!(send["ok"], true);
    assert_eq!(send["payload"]["status"], "completed");

    let history = rpc_req(
        &mut ws,
        "chat-2",
        "chat.history",
        Some(json!({ "sessionKey": "agent:main:main", "limit": 10 })),
    )
    .await;
    assert_eq!(history["ok"], true);
    assert!(
        history["payload"]["messages"]
            .as_array()
            .is_some_and(|messages| messages.len() >= 2)
    );

    let wait = rpc_req(
        &mut ws,
        "agent-1",
        "agent.wait",
        Some(json!({ "runId": "run-chat-1", "timeoutMs": 500 })),
    )
    .await;
    assert_eq!(wait["ok"], true);
    assert_eq!(wait["payload"]["status"], "completed");

    let add_job = rpc_req(
        &mut ws,
        "cron-1",
        "cron.add",
        Some(json!({
            "id": "job-1",
            "name": "Job One",
            "enabled": true,
            "schedule": { "kind": "every", "everyMs": 1000 },
            "payload": { "kind": "systemEvent", "text": "tick" },
            "metadata": {}
        })),
    )
    .await;
    assert_eq!(add_job["ok"], true);

    let run_job = rpc_req(
        &mut ws,
        "cron-2",
        "cron.run",
        Some(json!({ "id": "job-1" })),
    )
    .await;
    assert_eq!(run_job["ok"], true);

    let runs = rpc_req(
        &mut ws,
        "cron-3",
        "cron.runs",
        Some(json!({ "id": "job-1", "limit": 10 })),
    )
    .await;
    assert_eq!(runs["ok"], true);
    assert!(
        runs["payload"]["count"]
            .as_u64()
            .is_some_and(|count| count >= 1)
    );

    let pair_request = rpc_req(
        &mut ws,
        "node-1",
        "node.pair.request",
        Some(json!({
            "nodeId": "node-a",
            "displayName": "Node A",
            "platform": "ios",
            "commands": ["ping"]
        })),
    )
    .await;
    assert_eq!(pair_request["ok"], true);

    let request_id = pair_request["payload"]["request"]["requestId"]
        .as_str()
        .expect("request id should exist")
        .to_owned();

    let approve = rpc_req(
        &mut ws,
        "node-2",
        "node.pair.approve",
        Some(json!({ "requestId": request_id })),
    )
    .await;
    assert_eq!(approve["ok"], true);

    let node_list = rpc_req(&mut ws, "node-3", "node.list", None).await;
    assert_eq!(node_list["ok"], true);

    let rename = rpc_req(
        &mut ws,
        "node-4",
        "node.rename",
        Some(json!({ "nodeId": "node-a", "displayName": "Node Renamed" })),
    )
    .await;
    assert_eq!(rename["ok"], true);

    let invoke = rpc_req(
        &mut ws,
        "node-5",
        "node.invoke",
        Some(json!({
            "nodeId": "node-a",
            "command": "test.command",
            "args": ["a"]
        })),
    )
    .await;
    assert_eq!(invoke["ok"], true);

    let invoke_id = invoke["payload"]["requestId"]
        .as_str()
        .expect("invoke id should exist")
        .to_owned();

    let invoke_result = rpc_req(
        &mut ws,
        "node-6",
        "node.invoke.result",
        Some(json!({
            "requestId": invoke_id,
            "status": "completed",
            "payload": { "ok": true }
        })),
    )
    .await;
    assert_eq!(invoke_result["ok"], true);

    let node_event = rpc_req(
        &mut ws,
        "node-7",
        "node.event",
        Some(json!({
            "nodeId": "node-a",
            "event": "heartbeat",
            "payload": { "ok": true }
        })),
    )
    .await;
    assert_eq!(node_event["ok"], true);

    let remove_job = rpc_req(
        &mut ws,
        "cron-4",
        "cron.remove",
        Some(json!({ "id": "job-1" })),
    )
    .await;
    assert_eq!(remove_job["ok"], true);
    assert_eq!(remove_job["payload"]["removed"], true);

    server.stop().await;
}

#[tokio::test]
async fn healthz_endpoint_returns_ok_payload() {
    let server = spawn_server(AuthMode::None).await;

    let response = reqwest::get(format!("http://{}/healthz", server.addr))
        .await
        .expect("healthz endpoint should respond");

    assert!(response.status().is_success());

    let payload: Value = response.json().await.expect("healthz should return json");
    assert_eq!(payload["ok"], true);

    server.stop().await;
}
