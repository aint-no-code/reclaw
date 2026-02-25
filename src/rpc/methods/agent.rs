use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};

use crate::{
    application::state::SharedState,
    domain::models::{AgentRunRecord, ChatMessage},
    rpc::{
        SessionContext,
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
    storage::now_unix_ms,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentRunParams {
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    idempotency_key: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    session_key: Option<String>,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentWaitParams {
    run_id: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentIdentityParams {
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    session_key: Option<String>,
}

pub async fn handle_agent(
    state: &SharedState,
    _session: &SessionContext,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: AgentRunParams = parse_required_params("agent", params)?;

    let input = parsed
        .input
        .or(parsed.message)
        .or(parsed.text)
        .and_then(trim_non_empty)
        .ok_or_else(|| {
            crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "invalid agent params: input is required",
            )
        })?;

    let run_id = parsed
        .run_id
        .and_then(trim_non_empty)
        .or_else(|| parsed.idempotency_key.and_then(trim_non_empty))
        .unwrap_or_else(|| format!("run-{}", uuid::Uuid::new_v4()));

    let session_key = parsed
        .session_key
        .and_then(trim_non_empty)
        .unwrap_or_else(|| "agent:main:main".to_owned());

    let agent_id = parsed
        .agent_id
        .and_then(trim_non_empty)
        .unwrap_or_else(|| "main".to_owned());

    let now = now_unix_ms();
    let output = format!("Echo: {input}");

    let run = AgentRunRecord {
        id: run_id.clone(),
        agent_id,
        input: input.clone(),
        output: output.clone(),
        status: "completed".to_owned(),
        session_key: Some(session_key.clone()),
        metadata: json!({
            "runtime": "reclaw-server",
            "source": "agent",
        }),
        created_at_ms: now,
        updated_at_ms: now,
        completed_at_ms: Some(now),
    };

    state
        .upsert_agent_run(&run)
        .await
        .map_err(map_domain_error)?;

    let messages = vec![
        ChatMessage {
            id: format!("msg-{}", uuid::Uuid::new_v4()),
            role: "user".to_owned(),
            text: input,
            status: "final".to_owned(),
            ts: now,
            metadata: json!({ "runId": run_id }),
        },
        ChatMessage {
            id: format!("msg-{}", uuid::Uuid::new_v4()),
            role: "assistant".to_owned(),
            text: output.clone(),
            status: "final".to_owned(),
            ts: now.saturating_add(1),
            metadata: json!({ "runId": run_id }),
        },
    ];

    state
        .append_chat_messages(&session_key, &messages)
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "runId": run_id,
        "status": "ok",
        "summary": "completed",
        "result": {
            "output": output,
            "sessionKey": session_key,
        },
    }))
}

pub async fn handle_agent_wait(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: AgentWaitParams = parse_required_params("agent.wait", params)?;
    let run_id = trim_non_empty(parsed.run_id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid agent.wait params: runId is required",
        )
    })?;

    let timeout_ms = parsed.timeout_ms.unwrap_or(30_000).min(120_000);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        if let Some(run) = state
            .get_agent_run(&run_id)
            .await
            .map_err(map_domain_error)?
        {
            return Ok(json!({
                "runId": run_id,
                "status": run.status,
                "startedAt": run.created_at_ms,
                "endedAt": run.completed_at_ms,
                "error": if run.status == "error" { Some(run.output) } else { None::<String> },
            }));
        }

        if Instant::now() >= deadline {
            return Ok(json!({
                "runId": run_id,
                "status": "timeout",
            }));
        }

        sleep(Duration::from_millis(50)).await;
    }
}

pub async fn handle_agent_identity(
    _state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: AgentIdentityParams = parse_optional_params("agent.identity.get", params)?;

    let agent_id = parsed
        .agent_id
        .and_then(trim_non_empty)
        .or_else(|| parsed.session_key.and_then(parse_agent_id_from_session_key))
        .unwrap_or_else(|| "main".to_owned());

    Ok(json!({
        "agentId": agent_id,
        "name": "Reclaw",
        "role": "assistant",
        "avatar": Value::Null,
        "runtime": "rust",
    }))
}

fn parse_agent_id_from_session_key(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut parts = trimmed.split(':');
    let prefix = parts.next()?;
    if prefix != "agent" {
        return None;
    }

    let agent_id = parts.next()?.trim();
    if agent_id.is_empty() {
        None
    } else {
        Some(agent_id.to_owned())
    }
}

fn trim_non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
