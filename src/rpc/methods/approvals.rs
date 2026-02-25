use std::hash::{Hash, Hasher};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::time::{Instant, sleep};

use crate::{
    application::state::SharedState,
    rpc::{
        SessionContext,
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
    storage::now_unix_ms,
};

const EXEC_APPROVALS_GLOBAL_KEY: &str = "runtime/exec-approvals/global";
const EXEC_APPROVALS_NODE_PREFIX: &str = "runtime/exec-approvals/node/";
const EXEC_APPROVAL_REQUEST_PREFIX: &str = "runtime/exec-approval/request/";
const DEFAULT_APPROVAL_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecApprovalsGetParams {
    #[serde(default)]
    base_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecApprovalsSetParams {
    file: Value,
    #[serde(default)]
    base_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecApprovalsNodeGetParams {
    node_id: String,
    #[serde(default)]
    base_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecApprovalsNodeSetParams {
    node_id: String,
    file: Value,
    #[serde(default)]
    base_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecApprovalRequest {
    command: String,
    cwd: Option<String>,
    node_id: Option<String>,
    host: Option<String>,
    security: Option<String>,
    ask: Option<String>,
    agent_id: Option<String>,
    resolved_path: Option<String>,
    session_key: Option<String>,
    requested_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecApprovalRecord {
    id: String,
    request: ExecApprovalRequest,
    status: String,
    decision: Option<String>,
    created_at_ms: u64,
    expires_at_ms: u64,
    resolved_at_ms: Option<u64>,
    resolved_by: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecApprovalRequestParams {
    #[serde(default)]
    id: Option<String>,
    command: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    security: Option<String>,
    #[serde(default)]
    ask: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    resolved_path: Option<String>,
    #[serde(default)]
    session_key: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    two_phase: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecApprovalWaitParams {
    id: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecApprovalResolveParams {
    id: String,
    decision: String,
}

pub async fn handle_exec_approvals_get(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ExecApprovalsGetParams = parse_optional_params("exec.approvals.get", params)?;
    let _ = parsed.base_hash;
    read_approvals_snapshot(state, EXEC_APPROVALS_GLOBAL_KEY).await
}

pub async fn handle_exec_approvals_set(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ExecApprovalsSetParams = parse_required_params("exec.approvals.set", params)?;
    save_approvals_snapshot(
        state,
        EXEC_APPROVALS_GLOBAL_KEY,
        parsed.file,
        parsed.base_hash,
        "exec.approvals.set",
    )
    .await
}

pub async fn handle_exec_approvals_node_get(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ExecApprovalsNodeGetParams =
        parse_required_params("exec.approvals.node.get", params)?;
    let node_id = trim_non_empty(parsed.node_id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid exec.approvals.node.get params: nodeId is required",
        )
    })?;
    let _ = parsed.base_hash;

    let key = format!("{EXEC_APPROVALS_NODE_PREFIX}{node_id}");
    read_approvals_snapshot(state, &key).await
}

pub async fn handle_exec_approvals_node_set(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ExecApprovalsNodeSetParams =
        parse_required_params("exec.approvals.node.set", params)?;
    let node_id = trim_non_empty(parsed.node_id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid exec.approvals.node.set params: nodeId is required",
        )
    })?;

    let key = format!("{EXEC_APPROVALS_NODE_PREFIX}{node_id}");
    save_approvals_snapshot(
        state,
        &key,
        parsed.file,
        parsed.base_hash,
        "exec.approvals.node.set",
    )
    .await
}

pub async fn handle_exec_approval_request(
    state: &SharedState,
    session: &SessionContext,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ExecApprovalRequestParams = parse_required_params("exec.approval.request", params)?;
    let command = trim_non_empty(parsed.command).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid exec.approval.request params: command is required",
        )
    })?;

    let host = parsed.host.and_then(trim_non_empty);
    let node_id = parsed.node_id.and_then(trim_non_empty);
    if host.as_deref() == Some("node") && node_id.is_none() {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "nodeId is required for host=node",
        ));
    }

    let id = parsed
        .id
        .and_then(trim_non_empty)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    if load_approval_record(state, &id).await?.is_some() {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "approval id already exists",
        ));
    }

    let timeout_ms = parsed
        .timeout_ms
        .unwrap_or(DEFAULT_APPROVAL_TIMEOUT_MS)
        .clamp(1_000, 300_000);
    let created_at_ms = now_unix_ms();
    let record = ExecApprovalRecord {
        id: id.clone(),
        request: ExecApprovalRequest {
            command,
            cwd: parsed.cwd.and_then(trim_non_empty),
            node_id,
            host,
            security: parsed.security.and_then(trim_non_empty),
            ask: parsed.ask.and_then(trim_non_empty),
            agent_id: parsed.agent_id.and_then(trim_non_empty),
            resolved_path: parsed.resolved_path.and_then(trim_non_empty),
            session_key: parsed.session_key.and_then(trim_non_empty),
            requested_by: Some(session.client_id.clone()),
        },
        status: "pending".to_owned(),
        decision: None,
        created_at_ms,
        expires_at_ms: created_at_ms.saturating_add(timeout_ms),
        resolved_at_ms: None,
        resolved_by: None,
    };

    save_approval_record(state, &record).await?;

    if parsed.two_phase.unwrap_or(false) {
        return Ok(json!({
            "status": "accepted",
            "id": record.id,
            "createdAtMs": record.created_at_ms,
            "expiresAtMs": record.expires_at_ms,
        }));
    }

    Ok(json!({
        "id": record.id,
        "decision": record.decision,
        "createdAtMs": record.created_at_ms,
        "expiresAtMs": record.expires_at_ms,
        "status": record.status,
    }))
}

pub async fn handle_exec_approval_wait_decision(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ExecApprovalWaitParams =
        parse_required_params("exec.approval.waitDecision", params)?;
    let id = trim_non_empty(parsed.id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid exec.approval.waitDecision params: id is required",
        )
    })?;

    let timeout_ms = parsed.timeout_ms.unwrap_or(15_000).clamp(1, 120_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        let Some(mut record) = load_approval_record(state, &id).await? else {
            return Err(crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "approval expired or not found",
            ));
        };

        if record.status != "pending" || record.decision.is_some() {
            return Ok(json!({
                "id": record.id,
                "decision": record.decision,
                "createdAtMs": record.created_at_ms,
                "expiresAtMs": record.expires_at_ms,
                "status": record.status,
            }));
        }

        if now_unix_ms() >= record.expires_at_ms {
            record.status = "expired".to_owned();
            save_approval_record(state, &record).await?;
            return Ok(json!({
                "id": record.id,
                "decision": Value::Null,
                "createdAtMs": record.created_at_ms,
                "expiresAtMs": record.expires_at_ms,
                "status": record.status,
            }));
        }

        if Instant::now() >= deadline {
            return Ok(json!({
                "id": record.id,
                "decision": Value::Null,
                "createdAtMs": record.created_at_ms,
                "expiresAtMs": record.expires_at_ms,
                "status": "pending",
            }));
        }

        sleep(Duration::from_millis(50)).await;
    }
}

pub async fn handle_exec_approval_resolve(
    state: &SharedState,
    session: &SessionContext,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ExecApprovalResolveParams = parse_required_params("exec.approval.resolve", params)?;
    let id = trim_non_empty(parsed.id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid exec.approval.resolve params: id is required",
        )
    })?;
    let decision = trim_non_empty(parsed.decision).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid exec.approval.resolve params: decision is required",
        )
    })?;

    if !matches!(decision.as_str(), "allow-once" | "allow-always" | "deny") {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid decision",
        ));
    }

    let Some(mut record) = load_approval_record(state, &id).await? else {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "unknown approval id",
        ));
    };

    if record.status != "pending" {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "approval is not pending",
        ));
    }

    record.status = "resolved".to_owned();
    record.decision = Some(decision.clone());
    record.resolved_at_ms = Some(now_unix_ms());
    record.resolved_by = Some(session.client_id.clone());
    save_approval_record(state, &record).await?;

    Ok(json!({
        "ok": true,
        "id": record.id,
        "decision": decision,
    }))
}

async fn read_approvals_snapshot(
    state: &SharedState,
    key: &str,
) -> Result<Value, crate::protocol::ErrorShape> {
    let file = state
        .get_config_entry_value(key)
        .await
        .map_err(map_domain_error)?;

    let exists = file.is_some();
    let file = file.unwrap_or_else(|| Value::Object(Map::new()));
    let hash = if exists {
        Some(stable_value_hash(&file))
    } else {
        None
    };

    Ok(json!({
        "path": key,
        "exists": exists,
        "hash": hash,
        "file": file,
    }))
}

async fn save_approvals_snapshot(
    state: &SharedState,
    key: &str,
    file: Value,
    base_hash: Option<String>,
    method: &str,
) -> Result<Value, crate::protocol::ErrorShape> {
    if !file.is_object() {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            format!("invalid {method} params: file must be an object"),
        ));
    }

    let current = read_approvals_snapshot(state, key).await?;
    let exists = current
        .get("exists")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let current_hash = current
        .get("hash")
        .and_then(Value::as_str)
        .map(str::to_owned);

    if exists {
        let Some(current_hash) = current_hash else {
            return Err(crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "exec approvals base hash unavailable; re-run get and retry",
            ));
        };
        let Some(base_hash) = base_hash.and_then(trim_non_empty) else {
            return Err(crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "exec approvals base hash required; re-run get and retry",
            ));
        };
        if base_hash != current_hash {
            return Err(crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "exec approvals changed since last load; re-run get and retry",
            ));
        }
    }

    let _ = state
        .set_config_entry_value(key, &file)
        .await
        .map_err(map_domain_error)?;

    read_approvals_snapshot(state, key).await
}

async fn load_approval_record(
    state: &SharedState,
    id: &str,
) -> Result<Option<ExecApprovalRecord>, crate::protocol::ErrorShape> {
    let key = format!("{EXEC_APPROVAL_REQUEST_PREFIX}{id}");
    let Some(raw) = state
        .get_config_entry_value(&key)
        .await
        .map_err(map_domain_error)?
    else {
        return Ok(None);
    };

    let record = serde_json::from_value::<ExecApprovalRecord>(raw).map_err(|error| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_UNAVAILABLE,
            format!("failed to decode approval record: {error}"),
        )
    })?;
    Ok(Some(record))
}

async fn save_approval_record(
    state: &SharedState,
    record: &ExecApprovalRecord,
) -> Result<(), crate::protocol::ErrorShape> {
    let key = format!("{EXEC_APPROVAL_REQUEST_PREFIX}{}", record.id);
    let payload = serde_json::to_value(record).map_err(|error| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_UNAVAILABLE,
            format!("failed to encode approval record: {error}"),
        )
    })?;

    let _ = state
        .set_config_entry_value(&key, &payload)
        .await
        .map_err(map_domain_error)?;
    Ok(())
}

fn stable_value_hash(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn trim_non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
