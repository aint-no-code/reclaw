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

const RUN_STATUS_QUEUED: &str = "queued";
const RUN_STATUS_RUNNING: &str = "running";
const RUN_STATUS_COMPLETED: &str = "completed";
const RUN_STATUS_ERROR: &str = "error";

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
    #[serde(default)]
    deferred: Option<bool>,
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
    let deferred = parsed.deferred.unwrap_or(false);

    if let Some(existing) = state
        .get_agent_run(&run_id)
        .await
        .map_err(map_domain_error)?
    {
        return resolve_existing_agent_run(existing, &session_key, &agent_id);
    }

    let now = now_unix_ms();
    let mut run = AgentRunRecord {
        id: run_id.clone(),
        agent_id: agent_id.clone(),
        input,
        output: String::new(),
        status: if deferred {
            RUN_STATUS_QUEUED.to_owned()
        } else {
            RUN_STATUS_RUNNING.to_owned()
        },
        session_key: Some(session_key.clone()),
        metadata: agent_run_metadata(deferred),
        created_at_ms: now,
        updated_at_ms: now,
        completed_at_ms: None,
    };

    if deferred {
        state
            .upsert_agent_run(&run)
            .await
            .map_err(map_domain_error)?;
        return Ok(agent_method_response(
            &run_id,
            &session_key,
            None,
            RUN_STATUS_QUEUED,
        ));
    }

    run = execute_agent_run(state, run).await?;
    Ok(agent_method_response(
        &run_id,
        &session_key,
        Some(run.output.as_str()),
        RUN_STATUS_COMPLETED,
    ))
}

fn agent_run_metadata(deferred: bool) -> Value {
    json!({
        "runtime": "reclaw-core",
        "source": "agent",
        "lineage": "openclaw",
        "deferred": deferred,
    })
}

fn agent_method_response(
    run_id: &str,
    session_key: &str,
    output: Option<&str>,
    summary: &str,
) -> Value {
    json!({
        "runId": run_id,
        "status": "ok",
        "summary": summary,
        "result": {
            "output": output.map(Value::from).unwrap_or(Value::Null),
            "sessionKey": session_key,
        },
    })
}

async fn execute_agent_run(
    state: &SharedState,
    mut run: AgentRunRecord,
) -> Result<AgentRunRecord, crate::protocol::ErrorShape> {
    let Some(session_key) = run.session_key.clone() else {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid stored agent run: sessionKey is required",
        ));
    };

    if run.status != RUN_STATUS_RUNNING {
        run.status = RUN_STATUS_RUNNING.to_owned();
        run.updated_at_ms = now_unix_ms();
    }
    state
        .upsert_agent_run(&run)
        .await
        .map_err(map_domain_error)?;

    let output = format!("Echo: {}", run.input);
    let messages = vec![
        ChatMessage {
            id: format!("msg-{}", uuid::Uuid::new_v4()),
            role: "user".to_owned(),
            text: run.input.clone(),
            status: "final".to_owned(),
            ts: run.updated_at_ms,
            metadata: json!({ "runId": run.id }),
        },
        ChatMessage {
            id: format!("msg-{}", uuid::Uuid::new_v4()),
            role: "assistant".to_owned(),
            text: output.clone(),
            status: "final".to_owned(),
            ts: run.updated_at_ms.saturating_add(1),
            metadata: json!({ "runId": run.id }),
        },
    ];

    if let Err(error) = state.append_chat_messages(&session_key, &messages).await {
        let failed_at = now_unix_ms();
        run.status = RUN_STATUS_ERROR.to_owned();
        run.output = format!("agent execution failed while appending chat messages: {error}");
        run.updated_at_ms = failed_at;
        run.completed_at_ms = Some(failed_at);
        state
            .upsert_agent_run(&run)
            .await
            .map_err(map_domain_error)?;
        return Err(map_domain_error(error));
    }

    let completed_at = now_unix_ms();
    run.status = RUN_STATUS_COMPLETED.to_owned();
    run.output = output;
    run.updated_at_ms = completed_at;
    run.completed_at_ms = Some(completed_at);
    state
        .upsert_agent_run(&run)
        .await
        .map_err(map_domain_error)?;

    Ok(run)
}

fn resolve_existing_agent_run(
    existing: AgentRunRecord,
    requested_session_key: &str,
    requested_agent_id: &str,
) -> Result<Value, crate::protocol::ErrorShape> {
    if existing
        .metadata
        .get("source")
        .and_then(Value::as_str)
        .is_some_and(|source| source != "agent")
    {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid agent params: runId is already used by another method",
        ));
    }

    if existing.agent_id != requested_agent_id {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid agent params: runId is already used with a different agentId",
        ));
    }

    if let Some(existing_session) = existing.session_key.as_deref()
        && existing_session != requested_session_key
    {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid agent params: runId is already used with a different sessionKey",
        ));
    }

    let output = if existing.status == RUN_STATUS_COMPLETED || existing.status == RUN_STATUS_ERROR {
        Value::from(existing.output.clone())
    } else {
        Value::Null
    };

    Ok(json!({
        "runId": existing.id,
        "status": "ok",
        "summary": existing.status,
        "result": {
            "output": output,
            "sessionKey": existing
                .session_key
                .unwrap_or_else(|| requested_session_key.to_owned()),
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
            if run.status == RUN_STATUS_QUEUED {
                let run = execute_agent_run(state, run).await?;
                return Ok(agent_wait_payload(&run_id, &run));
            }
            if run.status == RUN_STATUS_RUNNING {
                if Instant::now() >= deadline {
                    return Ok(json!({
                        "runId": run_id,
                        "status": "timeout",
                    }));
                }
                sleep(Duration::from_millis(50)).await;
                continue;
            }

            return Ok(agent_wait_payload(&run_id, &run));
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

fn agent_wait_payload(run_id: &str, run: &AgentRunRecord) -> Value {
    let output = if run.status == RUN_STATUS_COMPLETED {
        Value::from(run.output.clone())
    } else {
        Value::Null
    };

    json!({
        "runId": run_id,
        "status": run.status,
        "startedAt": run.created_at_ms,
        "endedAt": run.completed_at_ms,
        "error": if run.status == RUN_STATUS_ERROR {
            Some(run.output.clone())
        } else {
            None::<String>
        },
        "result": {
            "output": output,
            "sessionKey": run.session_key,
        },
    })
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
