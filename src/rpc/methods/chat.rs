use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    application::state::SharedState,
    domain::models::{AgentRunRecord, ChatMessage, SessionRecord},
    rpc::{
        SessionContext,
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
    storage::now_unix_ms,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatSendParams {
    #[serde(default)]
    session_key: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    message: String,
    #[serde(default)]
    idempotency_key: Option<String>,
    #[serde(default)]
    deferred: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatHistoryParams {
    #[serde(default)]
    session_key: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatAbortParams {
    #[serde(default)]
    session_key: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
}

pub async fn handle_send(
    state: &SharedState,
    session: &SessionContext,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ChatSendParams = parse_required_params("chat.send", params)?;

    let session_key = resolve_session_key(parsed.session_key, parsed.session_id)?;
    let inbound = sanitize_chat_message(parsed.message)?;
    let deferred = parsed.deferred.unwrap_or(false);

    let run_id = parsed
        .idempotency_key
        .and_then(trim_non_empty)
        .unwrap_or_else(|| format!("chat-{}", uuid::Uuid::new_v4()));

    if let Some(existing) = state
        .get_agent_run(&run_id)
        .await
        .map_err(map_domain_error)?
    {
        return resolve_existing_chat_run(existing, &session_key);
    }

    ensure_session_exists(state, &session_key).await?;

    let now = now_unix_ms();
    if deferred {
        let run = AgentRunRecord {
            id: run_id.clone(),
            agent_id: "main".to_owned(),
            input: inbound,
            output: String::new(),
            status: "queued".to_owned(),
            session_key: Some(session_key.clone()),
            metadata: json!({
                "source": "chat.send",
                "deferred": true,
                "originConnId": session.conn_id.as_str(),
            }),
            created_at_ms: now,
            updated_at_ms: now,
            completed_at_ms: None,
        };

        state
            .upsert_agent_run(&run)
            .await
            .map_err(map_domain_error)?;

        return Ok(json!({
            "runId": run_id,
            "status": "queued",
            "sessionKey": session_key,
            "message": Value::Null,
        }));
    }

    let reply = format!("Echo: {inbound}");

    let messages = vec![
        ChatMessage {
            id: format!("msg-{}", uuid::Uuid::new_v4()),
            role: "user".to_owned(),
            text: inbound.clone(),
            status: "final".to_owned(),
            ts: now,
            metadata: json!({ "runId": run_id }),
        },
        ChatMessage {
            id: format!("msg-{}", uuid::Uuid::new_v4()),
            role: "assistant".to_owned(),
            text: reply.clone(),
            status: "final".to_owned(),
            ts: now.saturating_add(1),
            metadata: json!({ "runId": run_id }),
        },
    ];

    state
        .append_chat_messages(&session_key, &messages)
        .await
        .map_err(map_domain_error)?;

    let run = AgentRunRecord {
        id: run_id.clone(),
        agent_id: "main".to_owned(),
        input: inbound,
        output: reply.clone(),
        status: "completed".to_owned(),
        session_key: Some(session_key.clone()),
        metadata: json!({
            "source": "chat.send",
            "deferred": false,
            "originConnId": session.conn_id.as_str(),
        }),
        created_at_ms: now,
        updated_at_ms: now,
        completed_at_ms: Some(now),
    };

    state
        .upsert_agent_run(&run)
        .await
        .map_err(map_domain_error)?;

    publish_chat_final_event(
        state,
        Some(session.conn_id.as_str()),
        &run_id,
        &session_key,
        &reply,
        now,
    )
    .await;

    Ok(json!({
        "runId": run_id,
        "status": "completed",
        "sessionKey": session_key,
        "message": reply,
    }))
}

async fn publish_chat_final_event(
    state: &SharedState,
    target_conn_id: Option<&str>,
    run_id: &str,
    session_key: &str,
    reply: &str,
    timestamp: u64,
) {
    state
        .publish_gateway_event_for(
            target_conn_id,
            "chat",
            json!({
                "runId": run_id,
                "sessionKey": session_key,
                "state": "final",
                "seq": 1,
                "message": {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": reply }],
                    "timestamp": timestamp,
                },
            }),
        )
        .await;
}

fn resolve_existing_chat_run(
    existing: AgentRunRecord,
    requested_session_key: &str,
) -> Result<Value, crate::protocol::ErrorShape> {
    if existing
        .metadata
        .get("source")
        .and_then(Value::as_str)
        .is_some_and(|source| source != "chat.send")
    {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid chat.send params: idempotency key already used by another method",
        ));
    }

    if let Some(existing_session) = existing.session_key.as_deref()
        && existing_session != requested_session_key
    {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid chat.send params: idempotency key already used with a different sessionKey",
        ));
    }

    Ok(json!({
        "runId": existing.id,
        "status": existing.status,
        "sessionKey": existing
            .session_key
            .unwrap_or_else(|| requested_session_key.to_owned()),
        "message": if existing.status == "completed" || existing.status == "error" {
            Value::from(existing.output)
        } else {
            Value::Null
        },
    }))
}

pub async fn handle_history(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ChatHistoryParams = parse_required_params("chat.history", params)?;
    let session_key = resolve_session_key(parsed.session_key, parsed.session_id)?;
    let limit = parsed.limit.map(|value| value.clamp(1, 1_000));

    let messages = state
        .list_chat_messages(&session_key, limit)
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "sessionKey": session_key,
        "sessionId": session_key,
        "messages": messages,
    }))
}

pub async fn handle_abort(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ChatAbortParams = parse_optional_params("chat.abort", params)?;
    let session_key = parsed
        .session_key
        .or(parsed.session_id)
        .and_then(trim_non_empty)
        .unwrap_or_else(|| "agent:main:main".to_owned());
    let Some(run_id) = parsed.run_id.and_then(trim_non_empty) else {
        let runs = state
            .list_agent_runs_by_session(&session_key, Some(500))
            .await
            .map_err(map_domain_error)?;
        let mut aborted_run_ids = Vec::new();
        for run in runs {
            if is_terminal_run_status(run.status.as_str()) {
                continue;
            }
            let aborted_run_id = run.id.clone();
            abort_run(state, run).await?;
            aborted_run_ids.push(aborted_run_id);
        }
        return Ok(json!({
            "ok": true,
            "aborted": !aborted_run_ids.is_empty(),
            "sessionKey": session_key,
            "runIds": aborted_run_ids,
        }));
    };

    let Some(run) = state
        .get_agent_run(&run_id)
        .await
        .map_err(map_domain_error)?
    else {
        return Ok(json!({
            "ok": true,
            "aborted": false,
            "sessionKey": session_key,
            "runIds": vec![run_id],
        }));
    };

    if run
        .session_key
        .as_deref()
        .is_some_and(|existing| existing != session_key)
    {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid chat.abort params: runId does not belong to sessionKey",
        ));
    }

    if is_terminal_run_status(run.status.as_str()) {
        return Ok(json!({
            "ok": true,
            "aborted": false,
            "sessionKey": session_key,
            "runIds": vec![run_id],
        }));
    }

    abort_run(state, run).await?;

    Ok(json!({
        "ok": true,
        "aborted": true,
        "sessionKey": session_key,
        "runIds": vec![run_id],
    }))
}

fn is_terminal_run_status(status: &str) -> bool {
    status == "completed" || status == "error" || status == "aborted"
}

async fn abort_run(
    state: &SharedState,
    mut run: AgentRunRecord,
) -> Result<(), crate::protocol::ErrorShape> {
    let aborted_at = now_unix_ms();
    run.status = "aborted".to_owned();
    run.updated_at_ms = aborted_at;
    run.completed_at_ms = Some(aborted_at);
    if run.output.trim().is_empty() {
        run.output = "aborted by chat.abort".to_owned();
    }
    if let Some(metadata) = run.metadata.as_object_mut() {
        metadata.insert("abortedBy".to_owned(), Value::from("chat.abort"));
        metadata.insert("abortedAtMs".to_owned(), Value::from(aborted_at));
    }
    state.upsert_agent_run(&run).await.map_err(map_domain_error)
}

fn resolve_session_key(
    session_key: Option<String>,
    session_id: Option<String>,
) -> Result<String, crate::protocol::ErrorShape> {
    let key = session_key
        .or(session_id)
        .and_then(trim_non_empty)
        .ok_or_else(|| {
            crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "invalid chat params: sessionKey is required",
            )
        })?;
    Ok(key)
}

fn sanitize_chat_message(input: String) -> Result<String, crate::protocol::ErrorShape> {
    if input.contains('\0') {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid chat.send params: message contains null bytes",
        ));
    }

    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid chat.send params: message or attachment required",
        ));
    }

    Ok(trimmed.to_owned())
}

async fn ensure_session_exists(
    state: &SharedState,
    session_key: &str,
) -> Result<(), crate::protocol::ErrorShape> {
    if state
        .get_session(session_key)
        .await
        .map_err(map_domain_error)?
        .is_some()
    {
        return Ok(());
    }

    let now = now_unix_ms();
    let session = SessionRecord {
        id: session_key.to_owned(),
        title: format!("Session {session_key}"),
        tags: Vec::new(),
        metadata: json!({}),
        created_at_ms: now,
        updated_at_ms: now,
    };

    state
        .upsert_session(&session)
        .await
        .map_err(map_domain_error)
}

fn trim_non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
