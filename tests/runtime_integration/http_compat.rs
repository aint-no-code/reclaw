use reclaw_core::application::config::AuthMode;
use serde_json::{Value, json};

use super::support::{spawn_server, spawn_server_with};

#[tokio::test]
async fn openai_chat_completions_requires_gateway_auth() {
    let server = spawn_server_with(AuthMode::Token("gateway-secret".to_owned()), |config| {
        config.openai_chat_completions_enabled = true;
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/v1/chat/completions", server.addr))
        .json(&json!({
            "model": "reclaw-core",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .expect("openai request should return");

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["error"]["type"], "authentication_error");

    server.stop().await;
}

#[tokio::test]
async fn openai_chat_completions_is_disabled_by_default() {
    let server = spawn_server(AuthMode::None).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/v1/chat/completions", server.addr))
        .json(&json!({
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .expect("openai request should return");

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

    server.stop().await;
}

#[tokio::test]
async fn openai_chat_completions_returns_openai_shape() {
    let server = spawn_server_with(AuthMode::Token("gateway-secret".to_owned()), |config| {
        config.openai_chat_completions_enabled = true;
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/v1/chat/completions", server.addr))
        .bearer_auth("gateway-secret")
        .json(&json!({
            "model": "support-agent",
            "user": "alice",
            "messages": [
                {"role": "system", "content": "you are concise"},
                {"role": "user", "content": "Summarize status"}
            ]
        }))
        .send()
        .await
        .expect("openai request should return");

    assert!(response.status().is_success());
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["object"], "chat.completion");
    assert_eq!(payload["model"], "support-agent");
    assert_eq!(payload["choices"][0]["message"]["role"], "assistant");
    assert!(
        payload["choices"][0]["message"]["content"]
            .as_str()
            .is_some_and(|value| value.contains("Echo:"))
    );

    server.stop().await;
}

#[tokio::test]
async fn openai_chat_completions_streams_sse_chunks() {
    let server = spawn_server_with(AuthMode::Token("gateway-secret".to_owned()), |config| {
        config.openai_chat_completions_enabled = true;
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/v1/chat/completions", server.addr))
        .bearer_auth("gateway-secret")
        .json(&json!({
            "model": "reclaw-core",
            "stream": true,
            "messages": [{"role": "user", "content": "stream me"}]
        }))
        .send()
        .await
        .expect("openai stream request should return");

    assert!(response.status().is_success());
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(content_type.starts_with("text/event-stream"));

    let body = response
        .text()
        .await
        .expect("response body should be readable");
    assert!(body.contains("chat.completion.chunk"));
    assert!(body.contains("data: [DONE]"));

    server.stop().await;
}

#[tokio::test]
async fn openresponses_requires_gateway_auth() {
    let server = spawn_server_with(AuthMode::Token("gateway-secret".to_owned()), |config| {
        config.openresponses_enabled = true;
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/v1/responses", server.addr))
        .json(&json!({
            "model": "reclaw-core",
            "input": "hello"
        }))
        .send()
        .await
        .expect("openresponses request should return");

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["error"]["type"], "authentication_error");

    server.stop().await;
}

#[tokio::test]
async fn openresponses_is_disabled_by_default() {
    let server = spawn_server(AuthMode::None).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/v1/responses", server.addr))
        .json(&json!({
            "input": "hello"
        }))
        .send()
        .await
        .expect("openresponses request should return");

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

    server.stop().await;
}

#[tokio::test]
async fn openresponses_returns_response_resource_shape() {
    let server = spawn_server_with(AuthMode::Token("gateway-secret".to_owned()), |config| {
        config.openresponses_enabled = true;
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/v1/responses", server.addr))
        .bearer_auth("gateway-secret")
        .json(&json!({
            "model": "support-agent",
            "user": "alice",
            "input": [
                {"role": "user", "content": [{"type": "input_text", "text": "Summarize status"}]}
            ]
        }))
        .send()
        .await
        .expect("openresponses request should return");

    assert!(response.status().is_success());
    let payload: Value = response.json().await.expect("response should be json");
    assert_eq!(payload["object"], "response");
    assert_eq!(payload["model"], "support-agent");
    assert_eq!(payload["status"], "completed");
    assert_eq!(payload["output"][0]["type"], "message");
    assert_eq!(payload["output"][0]["role"], "assistant");
    assert!(
        payload["output"][0]["content"][0]["text"]
            .as_str()
            .is_some_and(|value| value.contains("Echo:"))
    );

    server.stop().await;
}

#[tokio::test]
async fn openresponses_streams_sse_events() {
    let server = spawn_server_with(AuthMode::Token("gateway-secret".to_owned()), |config| {
        config.openresponses_enabled = true;
    })
    .await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{}/v1/responses", server.addr))
        .bearer_auth("gateway-secret")
        .json(&json!({
            "model": "reclaw-core",
            "stream": true,
            "input": "stream me"
        }))
        .send()
        .await
        .expect("openresponses stream request should return");

    assert!(response.status().is_success());
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(content_type.starts_with("text/event-stream"));

    let body = response
        .text()
        .await
        .expect("response body should be readable");
    assert!(body.contains("event: response.created"));
    assert!(body.contains("event: response.output_text.delta"));
    assert!(body.contains("event: response.completed"));
    assert!(body.contains("data: [DONE]"));

    server.stop().await;
}
