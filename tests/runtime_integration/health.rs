use reclaw_core::application::config::AuthMode;
use serde_json::Value;

use super::support::spawn_server;

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
