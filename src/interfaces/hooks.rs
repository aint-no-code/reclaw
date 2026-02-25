use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use axum::{
    Json,
    body::to_bytes,
    extract::{ConnectInfo, Path as AxumPath, Request, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::process::Command;
use tokio::time::timeout;

use crate::{
    application::{
        config::{HookMappingAction, HookMappingConfig, HookMappingTransformConfig, RuntimeConfig},
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
const HOOKS_TRANSFORM_CONTEXT_ENV: &str = "RECLAW_HOOK_CONTEXT_JSON";
const HOOKS_TRANSFORM_EXPORT_ENV: &str = "RECLAW_HOOK_TRANSFORM_EXPORT";
const HOOKS_TRANSFORM_TIMEOUT: Duration = Duration::from_secs(5);
const HOOKS_JS_TRANSFORM_RUNNER: &str = r#"
import { pathToFileURL } from 'node:url';

const modulePath = process.argv[1];
const exportName = process.argv[2] || '';
const context = process.env.RECLAW_HOOK_CONTEXT_JSON ?? '{}';

const mod = await import(pathToFileURL(modulePath).href);
const candidate = (exportName && mod[exportName]) ?? mod.default ?? mod.transform;
if (typeof candidate !== 'function') {
  throw new Error('hook transform module must export a function');
}

const parsed = JSON.parse(context);
const output = await candidate(parsed);
process.stdout.write(JSON.stringify(output === undefined ? null : output));
"#;

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

struct HookTemplateContext<'a> {
    payload: &'a Map<String, Value>,
    headers: &'a Map<String, Value>,
    path: &'a str,
    query: &'a Map<String, Value>,
    url: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HookTransformOverride {
    #[serde(default)]
    kind: Option<HookMappingAction>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    mode: Option<String>,
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
}

#[derive(Debug)]
enum HookResolvedAction {
    Wake(HookWakeNormalized),
    Agent(HookAgentNormalized),
}

pub async fn root_handler(
    State(state): State<SharedState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    request: Request,
) -> (StatusCode, Json<Value>) {
    handle_request(state, remote_addr, String::new(), request).await
}

pub async fn subpath_handler(
    AxumPath(subpath): AxumPath<String>,
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
    let request_uri = request.uri().clone();
    let request_headers = request.headers().clone();
    if request.method() != Method::POST {
        return error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "METHOD_NOT_ALLOWED",
            "method not allowed",
        );
    }

    if has_token_query(&request_uri) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
            HOOKS_QUERY_TOKEN_ERROR,
        );
    }

    if let Err(response) = authorize_request(&state, &request_headers, remote_addr).await {
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
    let normalized_headers = normalize_hook_headers(&request_headers);
    let query_values = parse_query_values(&request_uri);
    let request_url = request_uri.to_string();
    let template_context = HookTemplateContext {
        payload: &payload,
        headers: &normalized_headers,
        path: normalized_subpath,
        query: &query_values,
        url: &request_url,
    };

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
            dispatch_mapping(state, mapped, &template_context).await
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
    context: &HookTemplateContext<'_>,
) -> (StatusCode, Json<Value>) {
    let base = match build_mapping_action(&mapping, context) {
        Ok(value) => value,
        Err(error) => {
            return error_response(StatusCode::BAD_REQUEST, "INVALID_REQUEST", error);
        }
    };
    let resolved = match apply_mapping_transform_if_configured(&state, &mapping, context, base)
        .await
    {
        Ok(Some(value)) => value,
        Ok(None) => {
            return (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "skipped": true,
                })),
            );
        }
        Err(error) => return error_response(StatusCode::SERVICE_UNAVAILABLE, "UNAVAILABLE", error),
    };

    match resolved {
        HookResolvedAction::Wake(wake) => dispatch_wake(state, wake).await,
        HookResolvedAction::Agent(agent) => {
            dispatch_agent(state, agent, HookSessionKeySource::Mapping).await
        }
    }
}

fn build_mapping_action(
    mapping: &HookMappingConfig,
    context: &HookTemplateContext<'_>,
) -> Result<HookResolvedAction, String> {
    match mapping.action {
        HookMappingAction::Wake => {
            let text = trim_non_empty(resolve_mapped_text(mapping, context))
                .ok_or_else(|| "hook mapping requires text".to_owned())?;
            Ok(HookResolvedAction::Wake(HookWakeNormalized {
                text,
                mode: HookWakeMode::from_raw(mapping.wake_mode.as_deref()),
            }))
        }
        HookMappingAction::Agent => {
            let message = trim_non_empty(resolve_mapped_message(mapping, context))
                .ok_or_else(|| "hook mapping requires message".to_owned())?;
            Ok(HookResolvedAction::Agent(HookAgentNormalized {
                message,
                name: trim_non_empty(mapping.name.clone()).unwrap_or_else(|| "Hook".to_owned()),
                agent_id: trim_non_empty(mapping.agent_id.clone()),
                wake_mode: HookWakeMode::from_raw(mapping.wake_mode.as_deref()),
                session_key: trim_non_empty(mapping.session_key.clone()),
            }))
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
    let Some(mapping_path) = mapping_path_value(mapping) else {
        return false;
    };
    if normalize_mapping_path(&mapping_path) != target_path {
        return false;
    }

    let Some(match_source) = mapping_match_source_value(mapping) else {
        return true;
    };
    payload
        .get("source")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|source| source == match_source)
}

fn mapping_path_value(mapping: &HookMappingConfig) -> Option<String> {
    trim_non_empty(Some(mapping.path.clone())).or_else(|| {
        mapping
            .r#match
            .as_ref()
            .and_then(|rule| trim_non_empty(rule.path.clone()))
    })
}

fn mapping_match_source_value(mapping: &HookMappingConfig) -> Option<String> {
    trim_non_empty(mapping.match_source.clone()).or_else(|| {
        mapping
            .r#match
            .as_ref()
            .and_then(|rule| trim_non_empty(rule.source.clone()))
    })
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
    context: &HookTemplateContext<'_>,
) -> Option<String> {
    if let Some(template) = trim_non_empty(mapping.text_template.clone()) {
        return Some(render_template(&template, context));
    }
    mapping.text.clone()
}

fn resolve_mapped_message(
    mapping: &HookMappingConfig,
    context: &HookTemplateContext<'_>,
) -> Option<String> {
    if let Some(template) = trim_non_empty(mapping.message_template.clone()) {
        return Some(render_template(&template, context));
    }
    mapping.message.clone()
}

async fn apply_mapping_transform_if_configured(
    state: &SharedState,
    mapping: &HookMappingConfig,
    context: &HookTemplateContext<'_>,
    base: HookResolvedAction,
) -> Result<Option<HookResolvedAction>, String> {
    let Some(transform) = mapping.transform.as_ref() else {
        return Ok(Some(base));
    };

    let override_data = execute_hook_transform(state.config(), transform, context).await?;
    let Some(override_data) = override_data else {
        return Ok(None);
    };

    merge_hook_action(base, override_data).map(Some)
}

fn merge_hook_action(
    base: HookResolvedAction,
    override_data: HookTransformOverride,
) -> Result<HookResolvedAction, String> {
    let base_kind = match base {
        HookResolvedAction::Wake(_) => HookMappingAction::Wake,
        HookResolvedAction::Agent(_) => HookMappingAction::Agent,
    };
    let effective_kind = override_data.kind.unwrap_or(base_kind);

    match effective_kind {
        HookMappingAction::Wake => {
            let base_wake = match base {
                HookResolvedAction::Wake(value) => Some(value),
                HookResolvedAction::Agent(_) => None,
            };
            let text = trim_non_empty(override_data.text)
                .or_else(|| base_wake.as_ref().map(|value| value.text.clone()))
                .ok_or_else(|| "hook mapping requires text".to_owned())?;
            let mode = if override_data
                .mode
                .as_deref()
                .is_some_and(|value| value.trim() == "next-heartbeat")
            {
                HookWakeMode::NextHeartbeat
            } else if override_data.mode.is_some() {
                HookWakeMode::Now
            } else {
                base_wake
                    .as_ref()
                    .map(|value| value.mode)
                    .unwrap_or(HookWakeMode::Now)
            };
            Ok(HookResolvedAction::Wake(HookWakeNormalized { text, mode }))
        }
        HookMappingAction::Agent => {
            let base_agent = match base {
                HookResolvedAction::Wake(_) => None,
                HookResolvedAction::Agent(value) => Some(value),
            };
            let message = trim_non_empty(override_data.message)
                .or_else(|| base_agent.as_ref().map(|value| value.message.clone()))
                .ok_or_else(|| "hook mapping requires message".to_owned())?;
            let wake_mode = if override_data
                .wake_mode
                .as_deref()
                .is_some_and(|value| value.trim() == "next-heartbeat")
            {
                HookWakeMode::NextHeartbeat
            } else if override_data.wake_mode.is_some() {
                HookWakeMode::Now
            } else {
                base_agent
                    .as_ref()
                    .map(|value| value.wake_mode)
                    .unwrap_or(HookWakeMode::Now)
            };

            Ok(HookResolvedAction::Agent(HookAgentNormalized {
                message,
                name: trim_non_empty(override_data.name)
                    .or_else(|| base_agent.as_ref().map(|value| value.name.clone()))
                    .unwrap_or_else(|| "Hook".to_owned()),
                agent_id: trim_non_empty(override_data.agent_id).or_else(|| {
                    base_agent
                        .as_ref()
                        .and_then(|value| trim_non_empty(value.agent_id.clone()))
                }),
                wake_mode,
                session_key: trim_non_empty(override_data.session_key).or_else(|| {
                    base_agent
                        .as_ref()
                        .and_then(|value| trim_non_empty(value.session_key.clone()))
                }),
            }))
        }
    }
}

async fn execute_hook_transform(
    config: &RuntimeConfig,
    transform: &HookMappingTransformConfig,
    context: &HookTemplateContext<'_>,
) -> Result<Option<HookTransformOverride>, String> {
    let module_path =
        resolve_transform_module_path(&config.hooks_transforms_dir, transform.module.as_str())?;
    let context_payload = json!({
        "payload": context.payload,
        "headers": context.headers,
        "query": context.query,
        "path": context.path,
        "url": context.url,
    });
    let context_json = serde_json::to_string(&context_payload)
        .map_err(|error| format!("failed to encode hook transform context: {error}"))?;

    let mut command = build_transform_command(
        &module_path,
        trim_non_empty(transform.export.clone()).as_deref(),
    );
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.env(HOOKS_TRANSFORM_CONTEXT_ENV, context_json);
    if let Some(export) = trim_non_empty(transform.export.clone()) {
        command.env(HOOKS_TRANSFORM_EXPORT_ENV, export);
    }

    let output = timeout(HOOKS_TRANSFORM_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            format!(
                "hook transform timed out after {}s: {}",
                HOOKS_TRANSFORM_TIMEOUT.as_secs(),
                module_path.display()
            )
        })?
        .map_err(|error| format!("failed to execute hook transform: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!(
                "hook transform failed with status {}: {}",
                output
                    .status
                    .code()
                    .map_or_else(|| "unknown".to_owned(), |code| code.to_string()),
                module_path.display()
            )
        } else {
            format!(
                "hook transform failed for {}: {stderr}",
                module_path.display()
            )
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if stdout.is_empty() {
        return Err(format!(
            "hook transform returned empty output: {}",
            module_path.display()
        ));
    }

    let value: Value = serde_json::from_str(&stdout)
        .map_err(|error| format!("hook transform emitted invalid JSON: {error}"))?;
    if value.is_null() {
        return Ok(None);
    }

    serde_json::from_value::<HookTransformOverride>(value)
        .map(Some)
        .map_err(|error| format!("hook transform output shape is invalid: {error}"))
}

fn build_transform_command(module_path: &Path, export_name: Option<&str>) -> Command {
    if is_js_transform_module(module_path) {
        let mut command = Command::new("node");
        command.arg("--input-type=module");
        command.arg("-e");
        command.arg(HOOKS_JS_TRANSFORM_RUNNER);
        command.arg(module_path);
        command.arg(export_name.unwrap_or_default());
        return command;
    }

    let mut command = Command::new(module_path);
    if let Some(export_name) = export_name {
        command.arg("--export");
        command.arg(export_name);
    }
    command
}

fn is_js_transform_module(module_path: &Path) -> bool {
    module_path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|value| value == "js" || value == "mjs" || value == "cjs")
}

fn resolve_transform_module_path(transforms_dir: &Path, module: &str) -> Result<PathBuf, String> {
    let module = module.trim();
    if module.is_empty() {
        return Err("hook transform module path is required".to_owned());
    }

    let base_dir = normalize_lexical_path(&absolutize_path(transforms_dir));
    let candidate = {
        let raw = PathBuf::from(module);
        if raw.is_absolute() {
            raw
        } else {
            base_dir.join(raw)
        }
    };
    let resolved = normalize_lexical_path(&absolutize_path(&candidate));
    if !resolved.starts_with(&base_dir) {
        return Err(format!(
            "hook transform module path must be within {}: {}",
            base_dir.display(),
            module
        ));
    }

    if let (Some(base_real), Some(existing_real)) = (
        canonicalize_if_exists(&base_dir),
        existing_ancestor(&resolved).and_then(|path| canonicalize_if_exists(&path)),
    ) && !existing_real.starts_with(base_real)
    {
        return Err(format!(
            "hook transform module path must be within {}: {}",
            base_dir.display(),
            module
        ));
    }

    Ok(resolved)
}

fn canonicalize_if_exists(path: &Path) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }
    std::fs::canonicalize(path).ok()
}

fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = path.to_path_buf();
    loop {
        if current.exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn absolutize_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    match std::env::current_dir() {
        Ok(current) => current.join(path),
        Err(_) => path.to_path_buf(),
    }
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let popped = out.pop();
                if !popped {
                    out.push(component.as_os_str());
                }
            }
            Component::Normal(segment) => out.push(segment),
        }
    }
    out
}

fn render_template(template: &str, context: &HookTemplateContext<'_>) -> String {
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
        out.push_str(&resolve_template_expr(context, expr));
        cursor = close + 2;
    }
    out.push_str(&template[cursor..]);
    out
}

fn resolve_template_expr(context: &HookTemplateContext<'_>, expr: &str) -> String {
    if expr == "path" {
        return context.path.to_owned();
    }

    let (source, expr) = if let Some(rest) = expr.strip_prefix("payload.") {
        (context.payload, rest)
    } else if let Some(rest) = expr.strip_prefix("headers.") {
        (context.headers, rest)
    } else if let Some(rest) = expr.strip_prefix("query.") {
        (context.query, rest)
    } else {
        (context.payload, expr)
    };

    let segments = parse_template_segments(expr);
    let mut segments_iter = segments.into_iter();
    let Some(first) = segments_iter.next() else {
        return String::new();
    };

    let mut cursor = match first {
        TemplateSegment::Key(key) => source.get(&key),
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

fn normalize_hook_headers(headers: &HeaderMap) -> Map<String, Value> {
    let mut normalized = Map::new();
    for (name, value) in headers {
        let Ok(text) = value.to_str() else {
            continue;
        };
        normalized.insert(
            name.as_str().to_ascii_lowercase(),
            Value::String(text.to_owned()),
        );
    }
    normalized
}

fn parse_query_values(uri: &Uri) -> Map<String, Value> {
    let mut query_values = Map::new();
    let Some(query) = uri.query() else {
        return query_values;
    };

    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().map(str::trim).unwrap_or_default();
        if key.is_empty() {
            continue;
        }
        let value = parts.next().map(str::trim).unwrap_or_default();
        query_values.insert(key.to_owned(), Value::String(value.to_owned()));
    }
    query_values
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
        HOOKS_SESSION_POLICY_ERROR, HookSessionKeySource, HookTemplateContext, has_token_query,
        mapping_matches, normalize_mapping_path, render_template, resolve_session_key_policy,
        resolve_transform_module_path,
    };
    use crate::application::config::{HookMappingAction, HookMappingConfig, RuntimeConfig};

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
    fn mapping_match_supports_openclaw_style_match_fields() {
        let mapping = HookMappingConfig {
            id: Some("openclaw-style".to_owned()),
            path: String::new(),
            r#match: Some(crate::application::config::HookMappingMatchConfig {
                path: Some("github/push".to_owned()),
                source: Some("github".to_owned()),
            }),
            action: HookMappingAction::Agent,
            match_source: None,
            wake_mode: None,
            text: None,
            text_template: None,
            message: Some("ok".to_owned()),
            message_template: None,
            name: None,
            agent_id: None,
            session_key: None,
            transform: None,
        };
        let payload = serde_json::json!({
            "source": "github",
        })
        .as_object()
        .cloned()
        .unwrap_or_default();

        assert!(mapping_matches(&mapping, "github/push", &payload));
    }

    #[test]
    fn render_template_resolves_nested_payload_paths() {
        let payload = serde_json::json!({
            "repo": "reclaw",
            "actor": { "name": "jd" },
            "commits": [{ "id": "c1" }]
        });
        let payload = payload.as_object().cloned().unwrap_or_default();
        let headers = serde_json::Map::new();
        let query = serde_json::Map::new();
        let context = HookTemplateContext {
            payload: &payload,
            headers: &headers,
            path: "github/push",
            query: &query,
            url: "/hooks/github/push",
        };
        let rendered = render_template(
            "repo={{repo}} actor={{actor.name}} first={{commits[0].id}}",
            &context,
        );
        assert_eq!(rendered, "repo=reclaw actor=jd first=c1");
    }

    #[test]
    fn render_template_supports_headers_query_and_path_contexts() {
        let payload = serde_json::Map::new();
        let headers = serde_json::json!({
            "user-agent": "reclaw-test",
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        let query = serde_json::json!({
            "kind": "push",
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        let context = HookTemplateContext {
            payload: &payload,
            headers: &headers,
            path: "github/template",
            query: &query,
            url: "/hooks/github/template?kind=push",
        };

        let rendered = render_template(
            "ua={{headers.user-agent}} kind={{query.kind}} path={{path}}",
            &context,
        );
        assert_eq!(rendered, "ua=reclaw-test kind=push path=github/template");
    }

    #[test]
    fn transform_path_resolution_blocks_parent_traversal() {
        let transforms_dir = std::path::PathBuf::from("/tmp/reclaw-hooks/transforms");
        let result = resolve_transform_module_path(&transforms_dir, "../evil.mjs");
        assert!(result.is_err());
    }
}
