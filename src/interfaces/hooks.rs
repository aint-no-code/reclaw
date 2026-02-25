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
    application::{
        config::{HookMappingAction, HookMappingConfig, RuntimeConfig},
        state::SharedState,
    },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookSessionKeySource {
    Request,
    Mapping,
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
        "wake" => {
            let normalized = match normalize_wake_payload(&payload) {
                Ok(value) => value,
                Err(error) => {
                    return error_response(StatusCode::BAD_REQUEST, "INVALID_REQUEST", error);
                }
            };
            dispatch_wake(state, normalized).await
        }
        "agent" => {
            let normalized = match normalize_agent_payload(&payload) {
                Ok(value) => value,
                Err(error) => {
                    return error_response(StatusCode::BAD_REQUEST, "INVALID_REQUEST", error);
                }
            };
            dispatch_agent(state, normalized, HookSessionKeySource::Request).await
        }
        _ => {
            let Some(mapped) = resolve_mapping(&state, normalized_subpath, &payload) else {
                return error_response(StatusCode::NOT_FOUND, "NOT_FOUND", "not found");
            };
            dispatch_mapping(state, mapped, &payload).await
        }
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

async fn dispatch_wake(
    state: SharedState,
    normalized: HookWakeNormalized,
) -> (StatusCode, Json<Value>) {
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
        if let Err(error) = system::handle_wake(&state, &session, Some(&wake_params)).await {
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

async fn dispatch_agent(
    state: SharedState,
    normalized: HookAgentNormalized,
    source: HookSessionKeySource,
) -> (StatusCode, Json<Value>) {
    let session_key =
        match resolve_session_key_policy(state.config(), normalized.session_key, source) {
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
        if let Err(error) = system::handle_wake(&state, &session, Some(&wake_params)).await {
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
    let response = match methods::agent::handle_agent(&state, &session, Some(&params)).await {
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

async fn dispatch_mapping(
    state: SharedState,
    mapping: HookMappingConfig,
    payload: &Map<String, Value>,
) -> (StatusCode, Json<Value>) {
    match mapping.action {
        HookMappingAction::Wake => {
            let text = trim_non_empty(resolve_mapped_text(&mapping, payload))
                .ok_or_else(|| "hook mapping requires text".to_owned());
            let text = match text {
                Ok(value) => value,
                Err(error) => {
                    return error_response(StatusCode::BAD_REQUEST, "INVALID_REQUEST", error);
                }
            };
            let wake = HookWakeNormalized {
                text,
                mode: HookWakeMode::from_raw(mapping.wake_mode.as_deref()),
            };
            dispatch_wake(state, wake).await
        }
        HookMappingAction::Agent => {
            let message = trim_non_empty(resolve_mapped_message(&mapping, payload))
                .ok_or_else(|| "hook mapping requires message".to_owned());
            let message = match message {
                Ok(value) => value,
                Err(error) => {
                    return error_response(StatusCode::BAD_REQUEST, "INVALID_REQUEST", error);
                }
            };
            let agent = HookAgentNormalized {
                message,
                name: trim_non_empty(mapping.name.clone()).unwrap_or_else(|| "Hook".to_owned()),
                agent_id: trim_non_empty(mapping.agent_id.clone()),
                wake_mode: HookWakeMode::from_raw(mapping.wake_mode.as_deref()),
                session_key: trim_non_empty(mapping.session_key.clone()),
            };
            dispatch_agent(state, agent, HookSessionKeySource::Mapping).await
        }
    }
}

fn resolve_mapping(
    state: &SharedState,
    subpath: &str,
    payload: &Map<String, Value>,
) -> Option<HookMappingConfig> {
    let target = normalize_mapping_path(subpath);
    state
        .config()
        .hooks_mappings
        .iter()
        .find(|mapping| mapping_matches(mapping, &target, payload))
        .cloned()
}

fn mapping_matches(
    mapping: &HookMappingConfig,
    target_path: &str,
    payload: &Map<String, Value>,
) -> bool {
    if normalize_mapping_path(&mapping.path) != target_path {
        return false;
    }

    let Some(match_source) = trim_non_empty(mapping.match_source.clone()) else {
        return true;
    };
    payload
        .get("source")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|source| source == match_source)
}

fn normalize_mapping_path(path: &str) -> String {
    path.trim()
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.trim().is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn resolve_mapped_text(
    mapping: &HookMappingConfig,
    payload: &Map<String, Value>,
) -> Option<String> {
    if let Some(template) = trim_non_empty(mapping.text_template.clone()) {
        return Some(render_template(&template, payload));
    }
    mapping.text.clone()
}

fn resolve_mapped_message(
    mapping: &HookMappingConfig,
    payload: &Map<String, Value>,
) -> Option<String> {
    if let Some(template) = trim_non_empty(mapping.message_template.clone()) {
        return Some(render_template(&template, payload));
    }
    mapping.message.clone()
}

fn render_template(template: &str, payload: &Map<String, Value>) -> String {
    let mut out = String::new();
    let mut cursor = 0usize;
    while let Some(open_rel) = template[cursor..].find("{{") {
        let open = cursor + open_rel;
        out.push_str(&template[cursor..open]);
        let value_start = open + 2;
        let Some(close_rel) = template[value_start..].find("}}") else {
            out.push_str(&template[open..]);
            return out;
        };
        let close = value_start + close_rel;
        let expr = template[value_start..close].trim();
        out.push_str(&resolve_template_expr(payload, expr));
        cursor = close + 2;
    }
    out.push_str(&template[cursor..]);
    out
}

fn resolve_template_expr(payload: &Map<String, Value>, expr: &str) -> String {
    let segments = parse_template_segments(expr);
    let mut segments_iter = segments.into_iter();
    let Some(first) = segments_iter.next() else {
        return String::new();
    };

    let mut cursor = match first {
        TemplateSegment::Key(key) => payload.get(&key),
        TemplateSegment::Index(_) => None,
    };

    for segment in segments_iter {
        let Some(value) = cursor else {
            return String::new();
        };
        cursor = match segment {
            TemplateSegment::Key(key) => value.get(&key),
            TemplateSegment::Index(index) => value.get(index),
        };
    }
    let Some(value) = cursor else {
        return String::new();
    };

    if let Some(value) = value.as_str() {
        return value.to_owned();
    }
    if value.is_number() || value.is_boolean() {
        return value.to_string();
    }
    serde_json::to_string(value).unwrap_or_default()
}

#[derive(Debug, Clone)]
enum TemplateSegment {
    Key(String),
    Index(usize),
}

fn parse_template_segments(path: &str) -> Vec<TemplateSegment> {
    let mut segments = Vec::new();
    for part in path.split('.').filter(|part| !part.trim().is_empty()) {
        let mut remainder = part.trim();
        while !remainder.is_empty() {
            if let Some(open) = remainder.find('[') {
                let key = remainder[..open].trim();
                if !key.is_empty() {
                    segments.push(TemplateSegment::Key(key.to_owned()));
                }
                let Some(close) = remainder[open + 1..].find(']') else {
                    break;
                };
                let index_raw = remainder[open + 1..open + 1 + close].trim();
                if let Ok(index) = index_raw.parse::<usize>() {
                    segments.push(TemplateSegment::Index(index));
                }
                let next = open + 1 + close + 1;
                remainder = &remainder[next..];
            } else {
                segments.push(TemplateSegment::Key(remainder.to_owned()));
                break;
            }
        }
    }
    segments
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
    source: HookSessionKeySource,
) -> Result<String, String> {
    if let Some(session_key) = requested_session_key {
        if source == HookSessionKeySource::Request && !config.hooks_allow_request_session_key {
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
    use super::{
        HOOKS_SESSION_POLICY_ERROR, HookSessionKeySource, has_token_query, normalize_mapping_path,
        render_template, resolve_session_key_policy,
    };
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

        let result = resolve_session_key_policy(
            &config,
            Some("hook:custom".to_owned()),
            HookSessionKeySource::Request,
        );
        assert_eq!(result, Err(HOOKS_SESSION_POLICY_ERROR.to_owned()));
    }

    #[test]
    fn normalize_mapping_path_trims_and_collapses_slashes() {
        assert_eq!(normalize_mapping_path("/github/push/"), "github/push");
        assert_eq!(normalize_mapping_path("  github//push  "), "github/push");
    }

    #[test]
    fn render_template_resolves_nested_payload_paths() {
        let payload = serde_json::json!({
            "repo": "reclaw",
            "actor": { "name": "jd" },
            "commits": [{ "id": "c1" }]
        });
        let payload = payload.as_object().cloned().unwrap_or_default();
        let rendered = render_template(
            "repo={{repo}} actor={{actor.name}} first={{commits[0].id}}",
            &payload,
        );
        assert_eq!(rendered, "repo=reclaw actor=jd first=c1");
    }
}
