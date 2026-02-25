use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    application::state::SharedState,
    rpc::{
        SessionContext,
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
    storage::now_unix_ms,
};

const LAST_HEARTBEAT_KEY: &str = "system/last-heartbeat";
const HEARTBEATS_KEY: &str = "system/heartbeats";
const SYSTEM_EVENT_PREFIX: &str = "system/events/";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeartbeatsSetParams {
    #[serde(default)]
    heartbeats: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WakeParams {
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SystemEventParams {
    event: String,
    #[serde(default)]
    payload: Option<Value>,
}

pub async fn handle_last_heartbeat(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: serde_json::Map<String, Value> = parse_optional_params("last-heartbeat", params)?;

    Ok(state
        .get_config_entry_value(LAST_HEARTBEAT_KEY)
        .await
        .map_err(map_domain_error)?
        .unwrap_or_else(|| {
            json!({
                "ts": 0,
                "status": "none",
            })
        }))
}

pub async fn handle_set_heartbeats(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: HeartbeatsSetParams = parse_required_params("set-heartbeats", params)?;
    let heartbeats = parsed.heartbeats.ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid set-heartbeats params: heartbeats is required",
        )
    })?;

    let _ = state
        .set_config_entry_value(HEARTBEATS_KEY, &heartbeats)
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "ok": true,
        "heartbeats": heartbeats,
    }))
}

pub async fn handle_wake(
    state: &SharedState,
    session: &SessionContext,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: WakeParams = parse_optional_params("wake", params)?;

    let payload = json!({
        "ts": now_unix_ms(),
        "reason": parsed.reason.and_then(trim_non_empty).unwrap_or_else(|| "manual".to_owned()),
        "by": session.client_id,
        "role": session.role,
    });

    let _ = state
        .set_config_entry_value(LAST_HEARTBEAT_KEY, &payload)
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "ok": true,
        "heartbeat": payload,
    }))
}

pub async fn handle_system_presence(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: serde_json::Map<String, Value> = parse_optional_params("system-presence", params)?;
    let snapshot = state.snapshot().await.map_err(map_domain_error)?;

    Ok(json!({
        "presence": snapshot.presence,
        "stateVersion": snapshot.state_version,
        "uptimeMs": snapshot.uptime_ms,
    }))
}

pub async fn handle_system_event(
    state: &SharedState,
    session: &SessionContext,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: SystemEventParams = parse_required_params("system-event", params)?;
    let event = trim_non_empty(parsed.event).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid system-event params: event is required",
        )
    })?;

    let ts = now_unix_ms();
    let id = format!("{SYSTEM_EVENT_PREFIX}{ts}-{}", uuid::Uuid::new_v4());
    let payload = json!({
        "id": id,
        "event": event,
        "payload": parsed.payload,
        "ts": ts,
        "by": session.client_id,
    });

    let _ = state
        .set_config_entry_value(&id, &payload)
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "ok": true,
        "entry": payload,
    }))
}

fn trim_non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
