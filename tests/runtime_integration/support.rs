use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use futures_util::{SinkExt, StreamExt};
use reclaw_core::application::{
    config::{AuthMode, RuntimeConfig},
    startup,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

pub(crate) type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

pub(crate) struct ServerHandle {
    pub(crate) addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
    _temp_dir: TempDir,
}

impl ServerHandle {
    pub(crate) async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = self.join.await;
    }
}

pub(crate) async fn spawn_server(auth_mode: AuthMode) -> ServerHandle {
    spawn_server_with(auth_mode, |_: &mut RuntimeConfig| {}).await
}

pub(crate) async fn spawn_server_with(
    auth_mode: AuthMode,
    configure: impl FnOnce(&mut RuntimeConfig),
) -> ServerHandle {
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
    configure(&mut config);

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

pub(crate) async fn connect_gateway(addr: SocketAddr) -> WsStream {
    let (socket, _) = connect_async(format!("ws://{addr}/"))
        .await
        .expect("websocket should connect");
    socket
}

pub(crate) fn connect_frame(
    auth_token: Option<&str>,
    min_protocol: u32,
    max_protocol: u32,
    role: &str,
    client_id: &str,
    scopes: &[&str],
) -> Value {
    json!({
        "type": "req",
        "id": "connect-1",
        "method": "connect",
        "params": {
            "minProtocol": min_protocol,
            "maxProtocol": max_protocol,
            "client": {
                "id": client_id,
                "displayName": format!("Reclaw Test {client_id}"),
                "version": "0.0.1",
                "platform": "test",
                "mode": "cli"
            },
            "role": role,
            "scopes": scopes,
            "auth": {
                "token": auth_token
            }
        }
    })
}

pub(crate) async fn recv_json(ws: &mut WsStream) -> Value {
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

pub(crate) async fn rpc_req(
    ws: &mut WsStream,
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
