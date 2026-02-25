use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    application::state::SharedState,
    domain::models::{NodeInvokeInput, NodePairRequestInput},
    rpc::{
        SessionContext,
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
    storage::now_unix_ms,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodePairRequestParams {
    node_id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    device_family: Option<String>,
    #[serde(default)]
    commands: Option<Vec<String>>,
    #[serde(default)]
    public_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodePairResolveParams {
    request_id: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeVerifyParams {
    node_id: String,
    #[serde(default)]
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeRenameParams {
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    id: Option<String>,
    display_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeIdParams {
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeInvokeParams {
    node_id: String,
    command: String,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    input: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeInvokeResultParams {
    request_id: String,
    status: String,
    #[serde(default)]
    payload: Option<Value>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeEventParams {
    #[serde(default)]
    node_id: Option<String>,
    event: String,
    #[serde(default)]
    payload: Option<Value>,
}

pub async fn handle_pair_request(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: NodePairRequestParams = parse_required_params("node.pair.request", params)?;

    let node_id = trim_non_empty(parsed.node_id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid node.pair.request params: nodeId is required",
        )
    })?;

    let display_name = parsed
        .display_name
        .and_then(trim_non_empty)
        .unwrap_or_else(|| node_id.clone());

    let platform = parsed
        .platform
        .and_then(trim_non_empty)
        .unwrap_or_else(|| "unknown".to_owned());

    let request = state
        .add_node_pair_request(NodePairRequestInput {
            node_id,
            display_name,
            platform,
            device_family: parsed.device_family.and_then(trim_non_empty),
            commands: sanitize_items(parsed.commands.unwrap_or_default()),
            public_key: parsed.public_key.and_then(trim_non_empty),
        })
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "status": "pending",
        "created": true,
        "request": request,
    }))
}

pub async fn handle_pair_list(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: serde_json::Map<String, Value> = parse_optional_params("node.pair.list", params)?;
    let requests = state
        .list_node_pair_requests()
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "ts": now_unix_ms(),
        "requests": requests,
    }))
}

pub async fn handle_pair_approve(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    handle_pair_resolution(state, params, true, "node.pair.approve").await
}

pub async fn handle_pair_reject(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    handle_pair_resolution(state, params, false, "node.pair.reject").await
}

pub async fn handle_pair_verify(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: NodeVerifyParams = parse_required_params("node.pair.verify", params)?;
    let node_id = trim_non_empty(parsed.node_id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid node.pair.verify params: nodeId is required",
        )
    })?;

    let Some(node) = state.get_node(&node_id).await.map_err(map_domain_error)? else {
        return Ok(json!({
            "ok": true,
            "nodeId": node_id,
            "paired": false,
            "verified": false,
        }));
    };

    let has_token = parsed
        .token
        .and_then(trim_non_empty)
        .is_some_and(|value| !value.is_empty());

    Ok(json!({
        "ok": true,
        "nodeId": node_id,
        "paired": node.paired,
        "verified": node.paired && has_token,
    }))
}

pub async fn handle_rename(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: NodeRenameParams = parse_required_params("node.rename", params)?;
    let node_id = resolve_node_id(parsed.node_id, parsed.id, "node.rename")?;
    let display_name = trim_non_empty(parsed.display_name).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid node.rename params: displayName is required",
        )
    })?;

    let node = state
        .rename_node(&node_id, &display_name)
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "nodeId": node.id,
        "displayName": node.display_name,
    }))
}

pub async fn handle_list(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: serde_json::Map<String, Value> = parse_optional_params("node.list", params)?;
    let nodes = state.list_nodes().await.map_err(map_domain_error)?;

    Ok(json!({
        "ts": now_unix_ms(),
        "nodes": nodes,
    }))
}

pub async fn handle_describe(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: NodeIdParams = parse_required_params("node.describe", params)?;
    let node_id = resolve_node_id(parsed.node_id, parsed.id, "node.describe")?;

    let node = state
        .get_node(&node_id)
        .await
        .map_err(map_domain_error)?
        .ok_or_else(|| {
            crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "unknown nodeId",
            )
        })?;

    Ok(json!({
        "ts": now_unix_ms(),
        "nodeId": node.id,
        "displayName": node.display_name,
        "platform": node.platform,
        "deviceFamily": node.device_family,
        "commands": node.commands,
        "paired": node.paired,
        "status": node.status,
        "lastSeenMs": node.last_seen_ms,
        "metadata": node.metadata,
    }))
}

pub async fn handle_invoke(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: NodeInvokeParams = parse_required_params("node.invoke", params)?;

    let node_id = trim_non_empty(parsed.node_id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid node.invoke params: nodeId is required",
        )
    })?;

    let command = trim_non_empty(parsed.command).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid node.invoke params: command is required",
        )
    })?;

    let invoke = state
        .create_node_invoke(NodeInvokeInput {
            node_id: node_id.clone(),
            command: command.clone(),
            args: sanitize_items(parsed.args.unwrap_or_default()),
            input: parsed.input,
        })
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "ok": true,
        "nodeId": node_id,
        "command": command,
        "requestId": invoke.request_id,
        "status": invoke.status,
        "payload": invoke.result,
    }))
}

pub async fn handle_invoke_result(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: NodeInvokeResultParams = parse_required_params("node.invoke.result", params)?;

    let request_id = trim_non_empty(parsed.request_id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid node.invoke.result params: requestId is required",
        )
    })?;

    let status = trim_non_empty(parsed.status).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid node.invoke.result params: status is required",
        )
    })?;

    let updated = state
        .update_node_invoke_result(&request_id, status, parsed.payload, parsed.error)
        .await
        .map_err(map_domain_error)?;

    Ok(json!(updated))
}

pub async fn handle_event(
    state: &SharedState,
    session: &SessionContext,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: NodeEventParams = parse_required_params("node.event", params)?;

    let node_id = parsed
        .node_id
        .and_then(trim_non_empty)
        .or_else(|| {
            if session.role == "node" {
                Some(session.client_id.clone())
            } else {
                None
            }
        })
        .ok_or_else(|| {
            crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "invalid node.event params: nodeId is required",
            )
        })?;

    let event = trim_non_empty(parsed.event).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid node.event params: event is required",
        )
    })?;

    let record = state
        .add_node_event(node_id, event, parsed.payload)
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "ok": true,
        "event": record,
    }))
}

async fn handle_pair_resolution(
    state: &SharedState,
    params: Option<&Value>,
    approved: bool,
    method: &str,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: NodePairResolveParams = parse_required_params(method, params)?;

    let request_id = trim_non_empty(parsed.request_id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            format!("invalid {method} params: requestId is required"),
        )
    })?;

    let resolved = state
        .resolve_node_pair_request(
            &request_id,
            approved,
            parsed.reason.and_then(trim_non_empty),
        )
        .await
        .map_err(map_domain_error)?;

    Ok(json!(resolved))
}

fn resolve_node_id(
    node_id: Option<String>,
    id: Option<String>,
    method: &str,
) -> Result<String, crate::protocol::ErrorShape> {
    node_id.or(id).and_then(trim_non_empty).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            format!("invalid {method} params: nodeId is required"),
        )
    })
}

fn sanitize_items(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }

        let normalized = trimmed.to_owned();
        if !out.contains(&normalized) {
            out.push(normalized);
        }
    }
    out
}

fn trim_non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
