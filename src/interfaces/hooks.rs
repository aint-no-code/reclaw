use std::net::SocketAddr;

use axum::{
    Json,
    body::to_bytes,
    extract::{ConnectInfo, Path, Request, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    application::{config::RuntimeConfig, state::SharedState},
    protocol::ERROR_INVALID_REQUEST,
    rpc::{
        SessionContext,
        dispatcher::map_domain_error,
        methods::{self, system},
        policy,
    },
    storage::now_unix_ms,
};

const HOOKS_LAST_WAKE_KEY: &str = "hooks/last-wake";
const HOOKS_PENDING_WAKE_PREFIX: &str = "hooks/pending-wake/";
const HOOKS_AUTH_SCOPE_PREFIX: &str = "hooks-auth:";
const HOOKS_TOKEN_HEADER: &str = "x-openclaw-token";
const HOOKS_SESSION_POLICY_ERROR: &str = "sessionKey is disabled for external /hooks/agent payloads; set hooksAllowRequestSessionKey=true to enable";
const HOOKS_QUERY_TOKEN_ERROR: &str = "Hook token must be provided via Authorization: Bearer <token> or X-OpenClaw-Token header (query parameters are not allowed).";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookWakeMode {
    Now,
    NextHeartbeat,
}

impl HookWakeMode {
    fn from_raw(raw: Option<&str>) -> Self {
        if raw.is_some_and(|value| value.trim() == "next-heartbeat") {
            Self::NextHeartbeat
        } else {
            Self::Now
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Now => "now",
            Self::NextHeartbeat => "next-heartbeat",
        }
    }
}

#[derive(Debug)]
struct HookWakeNormalized {
    text: String,
    mode: HookWakeMode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HookAgentPayload {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    wake_mode: Option<String>,
    #[serde(default)]
    session_key: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug)]
struct HookAgentNormalized {
    message: String,
    name: String,
    agent_id: Option<String>,
    wake_mode: HookWakeMode,
    session_key: Option<String>,
}

pub async fn root_handler(
    State(state): State<SharedState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    request: Request,
) -> (StatusCode, Json<Value>) {
    handle_request(state, remote_addr, String::new(), request).await
}

pub async fn subpath_handler(
    Path(subpath): Path<String>,
    State(state): State<SharedState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    request: Request,
) -> (StatusCode, Json<Value>) {
    handle_request(state, remote_addr, subpath, request).await
}

async fn handle_request(
    state: SharedState,
    remote_addr: SocketAddr,
    subpath: String,
    request: Request,
) -> (StatusCode, Json<Value>) {
    if request.method() != Method::POST {
        return error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "METHOD_NOT_ALLOWED",
            "method not allowed",
        );
    }

    if has_token_query(request.uri()) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
            HOOKS_QUERY_TOKEN_ERROR,
        );
    }

    if let Err(response) = authorize_request(&state, request.headers(), remote_addr).await {
        return response;
    }

    let body = match to_bytes(request.into_body(), state.config().hooks_max_body_bytes).await {
        Ok(body) => body,
        Err(_) => {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "PAYLOAD_TOO_LARGE",
                "payload too large",
            );
        }
    };

    let parsed = if body.is_empty() {
        Value::Object(Map::new())
    } else {
        match serde_json::from_slice::<Value>(&body) {
            Ok(value) => value,
            Err(error) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "INVALID_REQUEST",
                    format!("invalid JSON payload: {error}"),
                );
            }
        }
    };
    let payload = parsed.as_object().cloned().unwrap_or_default();
    let normalized_subpath = subpath.trim_matches('/');
    if normalized_subpath.is_empty() {
        return error_response(StatusCode::NOT_FOUND, "NOT_FOUND", "not found");
    }

    match normalized_subpath {
        "wake" => handle_wake_hook(&state, &payload).await,
        "agent" => handle_agent_hook(&state, &payload).await,
        _ => error_response(StatusCode::NOT_FOUND, "NOT_FOUND", "not found"),
    }
}

async fn authorize_request(
    state: &SharedState,
    headers: &HeaderMap,
    remote_addr: SocketAddr,
) -> Result<(), (StatusCode, Json<Value>)> {
    let Some(expected_token) = state.config().hooks_token.as_deref() else {
        return Err(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "UNAVAILABLE",
            "hooks token is not configured",
        ));
    };

    let provided_token = extract_hook_token(headers);
    let rate_limit_key = format!("{HOOKS_AUTH_SCOPE_PREFIX}{}", remote_addr.ip());

    if !token_matches(provided_token, expected_token) {
        let decision = state
            .control_plane_rate_limiter()
            .record_failure(&rate_limit_key)
            .await;
        if !decision.allowed {
            return Err(error_response(
                StatusCode::TOO_MANY_REQUESTS,
                "RATE_LIMITED",
                "too many failed authentication attempts",
            ));
        }
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "unauthorized",
        ));
    }

    state
        .control_plane_rate_limiter()
        .reset(&rate_limit_key)
        .await;
    Ok(())
}

async fn handle_wake_hook(
    state: &SharedState,
    payload: &Map<String, Value>,
) -> (StatusCode, Json<Value>) {
    let normalized = match normalize_wake_payload(payload) {
        Ok(value) => value,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, "INVALID_REQUEST", error),
    };

    let wake_entry = json!({
        "ts": now_unix_ms(),
        "text": normalized.text,
        "mode": normalized.mode.as_str(),
    });
    if let Err(error) = state
        .set_config_entry_value(HOOKS_LAST_WAKE_KEY, &wake_entry)
        .await
        .map_err(map_domain_error)
    {
        return map_error_shape(error);
    }

    if normalized.mode == HookWakeMode::Now {
        let wake_params = json!({
            "reason": "hook:wake",
        });
        let session = hook_session("hooks-wake");
        if let Err(error) = system::handle_wake(state, &session, Some(&wake_params)).await {
            return map_error_shape(error);
        }
    } else {
        let pending_key = format!("{HOOKS_PENDING_WAKE_PREFIX}{}", uuid::Uuid::new_v4());
        if let Err(error) = state
            .set_config_entry_value(&pending_key, &wake_entry)
            .await
            .map_err(map_domain_error)
        {
            return map_error_shape(error);
        }
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "mode": normalized.mode.as_str(),
        })),
    )
}

async fn handle_agent_hook(
    state: &SharedState,
    payload: &Map<String, Value>,
) -> (StatusCode, Json<Value>) {
    let normalized = match normalize_agent_payload(payload) {
        Ok(value) => value,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, "INVALID_REQUEST", error),
    };

    let session_key =
        match resolve_session_key_policy(state.config(), normalized.session_key.clone()) {
            Ok(value) => value,
            Err(error) => return error_response(StatusCode::BAD_REQUEST, "INVALID_REQUEST", error),
        };
    let agent_id = normalized
        .agent_id
        .unwrap_or_else(|| state.config().hooks_default_agent_id.clone());

    if normalized.wake_mode == HookWakeMode::Now {
        let wake_params = json!({
            "reason": "hook:agent",
        });
        let session = hook_session("hooks-agent-wake");
        if let Err(error) = system::handle_wake(state, &session, Some(&wake_params)).await {
            return map_error_shape(error);
        }
    }

    let params = json!({
        "message": normalized.message,
        "name": normalized.name,
        "agentId": agent_id,
        "sessionKey": session_key,
        "idempotencyKey": format!("hook-agent-{}", uuid::Uuid::new_v4()),
    });
    let session = hook_session("hooks-agent");
    let response = match methods::agent::handle_agent(state, &session, Some(&params)).await {
        Ok(payload) => payload,
        Err(error) => return map_error_shape(error),
    };

    let run_id = response
        .get("runId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    (
        StatusCode::ACCEPTED,
        Json(json!({
            "ok": true,
            "runId": run_id,
            "sessionKey": session_key,
            "agentId": agent_id,
        })),
    )
}

fn normalize_wake_payload(payload: &Map<String, Value>) -> Result<HookWakeNormalized, String> {
    let text = payload
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "text required".to_owned())?
        .to_owned();

    let mode = HookWakeMode::from_raw(payload.get("mode").and_then(Value::as_str));
    Ok(HookWakeNormalized { text, mode })
}

fn normalize_agent_payload(payload: &Map<String, Value>) -> Result<HookAgentNormalized, String> {
    let raw = Value::Object(payload.clone());
    let parsed = serde_json::from_value::<HookAgentPayload>(raw)
        .map_err(|error| format!("invalid agent payload: {error}"))?;

    let message = trim_non_empty(parsed.message).ok_or_else(|| "message required".to_owned())?;
    let name = trim_non_empty(parsed.name).unwrap_or_else(|| "Hook".to_owned());
    let model = parsed.model;
    if model.as_ref().is_some_and(|value| value.trim().is_empty()) {
        return Err("model required".to_owned());
    }

    Ok(HookAgentNormalized {
        message,
        name,
        agent_id: trim_non_empty(parsed.agent_id),
        wake_mode: HookWakeMode::from_raw(parsed.wake_mode.as_deref()),
        session_key: trim_non_empty(parsed.session_key),
    })
}

fn resolve_session_key_policy(
    config: &RuntimeConfig,
    requested_session_key: Option<String>,
) -> Result<String, String> {
    if let Some(session_key) = requested_session_key {
        if !config.hooks_allow_request_session_key {
            return Err(HOOKS_SESSION_POLICY_ERROR.to_owned());
        }
        return Ok(session_key);
    }

    if let Some(default_key) = &config.hooks_default_session_key {
        return Ok(default_key.clone());
    }

    Ok(format!("hook:{}", uuid::Uuid::new_v4()))
}

fn hook_session(client_id: &str) -> SessionContext {
    SessionContext {
        conn_id: format!("{client_id}-{}", uuid::Uuid::new_v4()),
        role: "operator".to_owned(),
        scopes: policy::default_operator_scopes(),
        client_id: client_id.to_owned(),
        client_mode: "hooks-http".to_owned(),
    }
}

fn has_token_query(uri: &Uri) -> bool {
    uri.query().is_some_and(|query| {
        query.split('&').any(|entry| {
            entry
                .split('=')
                .next()
                .is_some_and(|key| key.eq_ignore_ascii_case("token"))
        })
    })
}

fn extract_hook_token(headers: &HeaderMap) -> Option<&str> {
    if let Some(value) = headers.get(header::AUTHORIZATION)
        && let Ok(auth) = value.to_str()
        && let Some(token) = auth
            .strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("bearer "))
    {
        let trimmed = token.trim();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }

    headers
        .get(HOOKS_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn token_matches(found: Option<&str>, expected: &str) -> bool {
    let Some(found) = found else {
        return false;
    };

    subtle::ConstantTimeEq::ct_eq(found.as_bytes(), expected.as_bytes()).into()
}

fn trim_non_empty(input: Option<String>) -> Option<String> {
    input.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn map_error_shape(error: crate::protocol::ErrorShape) -> (StatusCode, Json<Value>) {
    let status = if error.code == ERROR_INVALID_REQUEST {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({
            "ok": false,
            "error": error,
        })),
    )
}

fn error_response(
    status: StatusCode,
    code: &str,
    message: impl Into<String>,
) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({
            "ok": false,
            "error": {
                "code": code,
                "message": message.into(),
            },
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::{HOOKS_SESSION_POLICY_ERROR, has_token_query, resolve_session_key_policy};
    use crate::application::config::RuntimeConfig;

    #[test]
    fn token_query_detector_matches_token_field() {
        let uri: axum::http::Uri = "/hooks/agent?token=abc".parse().expect("uri should parse");
        assert!(has_token_query(&uri));

        let uri: axum::http::Uri = "/hooks/agent?a=1&b=2".parse().expect("uri should parse");
        assert!(!has_token_query(&uri));
    }

    #[test]
    fn session_policy_blocks_request_session_key_by_default() {
        let config = RuntimeConfig::for_test(
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            18789,
            std::path::PathBuf::from(":memory:"),
        );

        let result = resolve_session_key_policy(&config, Some("hook:custom".to_owned()));
        assert_eq!(result, Err(HOOKS_SESSION_POLICY_ERROR.to_owned()));
    }
}
