use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    application::state::SharedState,
    rpc::{
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
    storage::now_unix_ms,
};

const DEVICE_STATE_KEY: &str = "runtime/device/state";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DeviceState {
    #[serde(default)]
    pending: Vec<DevicePairRequest>,
    #[serde(default)]
    paired: Vec<PairedDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevicePairRequest {
    request_id: String,
    device_id: String,
    display_name: Option<String>,
    role: Option<String>,
    scopes: Vec<String>,
    created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceAuthToken {
    role: String,
    token: String,
    scopes: Vec<String>,
    created_at_ms: u64,
    rotated_at_ms: Option<u64>,
    revoked_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairedDevice {
    device_id: String,
    display_name: Option<String>,
    role: Option<String>,
    scopes: Vec<String>,
    approved_scopes: Vec<String>,
    paired_at_ms: u64,
    tokens: BTreeMap<String, DeviceAuthToken>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevicePairApproveParams {
    request_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevicePairRejectParams {
    request_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DevicePairRemoveParams {
    device_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceTokenRotateParams {
    device_id: String,
    role: String,
    #[serde(default)]
    scopes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceTokenRevokeParams {
    device_id: String,
    role: String,
}

pub async fn handle_pair_list(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: Map<String, Value> = parse_optional_params("device.pair.list", params)?;
    let mut current = load_device_state(state).await?;

    // Surface pending node pair requests as device-pair candidates.
    let node_requests = state
        .list_node_pair_requests()
        .await
        .map_err(map_domain_error)?;
    for node_request in node_requests {
        if node_request.status != "pending" {
            continue;
        }
        if current
            .pending
            .iter()
            .any(|request| request.request_id == node_request.request_id)
        {
            continue;
        }
        current.pending.push(DevicePairRequest {
            request_id: node_request.request_id,
            device_id: node_request.node_id,
            display_name: Some(node_request.display_name),
            role: Some("node".to_owned()),
            scopes: Vec::new(),
            created_at_ms: node_request.created_at_ms,
        });
    }

    Ok(json!({
        "pending": current.pending,
        "paired": current
            .paired
            .iter()
            .map(redact_paired_device)
            .collect::<Vec<_>>(),
    }))
}

pub async fn handle_pair_approve(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: DevicePairApproveParams = parse_required_params("device.pair.approve", params)?;
    let request_id = trim_non_empty(parsed.request_id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid device.pair.approve params: requestId is required",
        )
    })?;

    let mut current = load_device_state(state).await?;
    let mut approved: Option<PairedDevice> = None;

    if let Some(index) = current
        .pending
        .iter()
        .position(|entry| entry.request_id == request_id)
    {
        let pending = current.pending.remove(index);
        let now = now_unix_ms();
        let paired = PairedDevice {
            device_id: pending.device_id.clone(),
            display_name: pending.display_name,
            role: pending.role,
            scopes: pending.scopes.clone(),
            approved_scopes: pending.scopes,
            paired_at_ms: now,
            tokens: BTreeMap::new(),
        };
        insert_or_replace_paired(&mut current.paired, paired.clone());
        approved = Some(paired);
    } else if let Ok(node_request) = state
        .resolve_node_pair_request(&request_id, true, None)
        .await
    {
        let now = now_unix_ms();
        let paired = PairedDevice {
            device_id: node_request.node_id.clone(),
            display_name: Some(node_request.display_name.clone()),
            role: Some("node".to_owned()),
            scopes: Vec::new(),
            approved_scopes: Vec::new(),
            paired_at_ms: now,
            tokens: BTreeMap::new(),
        };
        insert_or_replace_paired(&mut current.paired, paired.clone());
        approved = Some(paired);
    }

    let Some(approved) = approved else {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "unknown requestId",
        ));
    };

    save_device_state(state, &current).await?;
    Ok(json!({
        "requestId": request_id,
        "device": redact_paired_device(&approved),
    }))
}

pub async fn handle_pair_reject(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: DevicePairRejectParams = parse_required_params("device.pair.reject", params)?;
    let request_id = trim_non_empty(parsed.request_id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid device.pair.reject params: requestId is required",
        )
    })?;

    let mut current = load_device_state(state).await?;
    let mut device_id: Option<String> = None;

    if let Some(index) = current
        .pending
        .iter()
        .position(|entry| entry.request_id == request_id)
    {
        let pending = current.pending.remove(index);
        device_id = Some(pending.device_id);
    } else if let Ok(node_request) = state
        .resolve_node_pair_request(&request_id, false, None)
        .await
    {
        device_id = Some(node_request.node_id);
    }

    let Some(device_id) = device_id else {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "unknown requestId",
        ));
    };

    save_device_state(state, &current).await?;
    Ok(json!({
        "ok": true,
        "requestId": request_id,
        "deviceId": device_id,
        "status": "rejected",
    }))
}

pub async fn handle_pair_remove(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: DevicePairRemoveParams = parse_required_params("device.pair.remove", params)?;
    let device_id = trim_non_empty(parsed.device_id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid device.pair.remove params: deviceId is required",
        )
    })?;

    let mut current = load_device_state(state).await?;
    let before = current.paired.len();
    current.paired.retain(|entry| entry.device_id != device_id);
    if current.paired.len() == before {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "unknown deviceId",
        ));
    }

    save_device_state(state, &current).await?;
    Ok(json!({
        "ok": true,
        "deviceId": device_id,
        "removedAtMs": now_unix_ms(),
    }))
}

pub async fn handle_token_rotate(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: DeviceTokenRotateParams = parse_required_params("device.token.rotate", params)?;
    let device_id = trim_non_empty(parsed.device_id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid device.token.rotate params: deviceId is required",
        )
    })?;
    let role = trim_non_empty(parsed.role).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid device.token.rotate params: role is required",
        )
    })?;

    let mut current = load_device_state(state).await?;
    let Some(device) = current
        .paired
        .iter_mut()
        .find(|entry| entry.device_id == device_id)
    else {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "unknown deviceId/role",
        ));
    };

    let scopes = sanitize_scopes(parsed.scopes.unwrap_or_else(|| device.scopes.clone()));
    let now = now_unix_ms();
    let token = format!("dtk_{}", uuid::Uuid::new_v4().simple());
    let entry = DeviceAuthToken {
        role: role.clone(),
        token: token.clone(),
        scopes: scopes.clone(),
        created_at_ms: now,
        rotated_at_ms: Some(now),
        revoked_at_ms: None,
    };
    device.tokens.insert(role.clone(), entry);

    save_device_state(state, &current).await?;
    Ok(json!({
        "deviceId": device_id,
        "role": role,
        "token": token,
        "scopes": scopes,
        "rotatedAtMs": now,
    }))
}

pub async fn handle_token_revoke(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: DeviceTokenRevokeParams = parse_required_params("device.token.revoke", params)?;
    let device_id = trim_non_empty(parsed.device_id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid device.token.revoke params: deviceId is required",
        )
    })?;
    let role = trim_non_empty(parsed.role).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid device.token.revoke params: role is required",
        )
    })?;

    let mut current = load_device_state(state).await?;
    let Some(device) = current
        .paired
        .iter_mut()
        .find(|entry| entry.device_id == device_id)
    else {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "unknown deviceId/role",
        ));
    };

    if device.tokens.remove(&role).is_none() {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "unknown deviceId/role",
        ));
    }

    let revoked_at_ms = now_unix_ms();
    save_device_state(state, &current).await?;
    Ok(json!({
        "deviceId": device_id,
        "role": role,
        "revokedAtMs": revoked_at_ms,
    }))
}

async fn load_device_state(
    state: &SharedState,
) -> Result<DeviceState, crate::protocol::ErrorShape> {
    let Some(raw) = state
        .get_config_entry_value(DEVICE_STATE_KEY)
        .await
        .map_err(map_domain_error)?
    else {
        return Ok(DeviceState::default());
    };

    serde_json::from_value::<DeviceState>(raw).map_err(|error| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_UNAVAILABLE,
            format!("failed to decode device state: {error}"),
        )
    })
}

async fn save_device_state(
    state: &SharedState,
    device_state: &DeviceState,
) -> Result<(), crate::protocol::ErrorShape> {
    let payload = serde_json::to_value(device_state).map_err(|error| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_UNAVAILABLE,
            format!("failed to encode device state: {error}"),
        )
    })?;

    let _ = state
        .set_config_entry_value(DEVICE_STATE_KEY, &payload)
        .await
        .map_err(map_domain_error)?;
    Ok(())
}

fn insert_or_replace_paired(entries: &mut Vec<PairedDevice>, candidate: PairedDevice) {
    if let Some(index) = entries
        .iter()
        .position(|entry| entry.device_id == candidate.device_id)
    {
        entries[index] = candidate;
    } else {
        entries.push(candidate);
    }
}

fn sanitize_scopes(values: Vec<String>) -> Vec<String> {
    let mut unique = BTreeMap::new();
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            unique.insert(trimmed.to_owned(), true);
        }
    }

    unique.into_keys().collect()
}

fn redact_paired_device(device: &PairedDevice) -> Value {
    let mut summarized = Vec::new();
    for entry in device.tokens.values() {
        summarized.push(json!({
            "role": entry.role,
            "scopes": entry.scopes,
            "createdAtMs": entry.created_at_ms,
            "rotatedAtMs": entry.rotated_at_ms,
            "revokedAtMs": entry.revoked_at_ms,
            "tokenPresent": !entry.token.is_empty(),
        }));
    }

    json!({
        "deviceId": device.device_id,
        "displayName": device.display_name,
        "role": device.role,
        "scopes": device.scopes,
        "approvedScopes": device.approved_scopes,
        "pairedAtMs": device.paired_at_ms,
        "tokens": summarized,
    })
}

fn trim_non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
