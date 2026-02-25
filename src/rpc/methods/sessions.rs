use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    application::state::SharedState,
    domain::models::SessionRecord,
    rpc::{
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
    storage::now_unix_ms,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionsListParams {
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionsPreviewParams {
    #[serde(default)]
    keys: Vec<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    max_chars: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionsPatchParams {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionsDeleteParams {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionsCompactParams {
    #[serde(default)]
    max_age_ms: Option<u64>,
}

pub async fn handle_list(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: SessionsListParams = parse_optional_params("sessions.list", params)?;
    let mut sessions = state.list_sessions().await.map_err(map_domain_error)?;

    if let Some(limit) = parsed.limit {
        sessions.truncate(limit);
    }

    Ok(json!({
        "ts": now_unix_ms(),
        "sessions": sessions,
    }))
}

pub async fn handle_preview(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: SessionsPreviewParams = parse_optional_params("sessions.preview", params)?;
    let max_keys = 64_usize;
    let limit = parsed.limit.unwrap_or(12).clamp(1, 200);
    let max_chars = parsed.max_chars.unwrap_or(240).clamp(20, 4_096);

    let mut previews = Vec::new();
    for key in parsed
        .keys
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .take(max_keys)
    {
        let session = state.get_session(&key).await.map_err(map_domain_error)?;
        let messages = state
            .list_chat_messages(&key, Some(limit))
            .await
            .map_err(map_domain_error)?;

        let items = messages
            .into_iter()
            .map(|message| {
                let mut text = message.text;
                if text.chars().count() > max_chars {
                    text = text.chars().take(max_chars).collect::<String>();
                }

                json!({
                    "id": message.id,
                    "role": message.role,
                    "text": text,
                    "status": message.status,
                    "ts": message.ts,
                })
            })
            .collect::<Vec<_>>();

        let status = if session.is_none() {
            "missing"
        } else if items.is_empty() {
            "empty"
        } else {
            "ok"
        };

        previews.push(json!({
            "key": key,
            "status": status,
            "items": items,
        }));
    }

    Ok(json!({
        "ts": now_unix_ms(),
        "previews": previews,
    }))
}

pub async fn handle_patch(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: SessionsPatchParams = parse_required_params("sessions.patch", params)?;
    let id = resolve_session_id(parsed.id, parsed.key)?;

    let existing = state.get_session(&id).await.map_err(map_domain_error)?;
    let now = now_unix_ms();

    let title = parsed
        .title
        .and_then(trim_non_empty)
        .or_else(|| existing.as_ref().map(|session| session.title.clone()))
        .unwrap_or_else(|| format!("Session {id}"));

    let tags = parsed
        .tags
        .map(|items| sanitize_tags(&items))
        .or_else(|| existing.as_ref().map(|session| session.tags.clone()))
        .unwrap_or_default();

    let metadata = match parsed.metadata {
        Some(value) if value.is_object() => value,
        Some(_) => {
            return Err(crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "invalid sessions.patch params: metadata must be an object",
            ));
        }
        None => existing
            .as_ref()
            .map(|session| session.metadata.clone())
            .unwrap_or_else(|| Value::Object(Map::new())),
    };

    let next = SessionRecord {
        id: id.clone(),
        title,
        tags,
        metadata,
        created_at_ms: existing
            .as_ref()
            .map_or(now, |session| session.created_at_ms),
        updated_at_ms: now,
    };

    state
        .upsert_session(&next)
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "ok": true,
        "key": id,
        "entry": next,
    }))
}

pub async fn handle_reset(state: &SharedState) -> Result<Value, crate::protocol::ErrorShape> {
    let removed = state.clear_sessions().await.map_err(map_domain_error)?;
    Ok(json!({
        "ok": true,
        "removed": removed,
    }))
}

pub async fn handle_delete(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: SessionsDeleteParams = parse_required_params("sessions.delete", params)?;
    let id = resolve_session_id(parsed.id, parsed.key)?;

    let deleted = state.remove_session(&id).await.map_err(map_domain_error)?;
    Ok(json!({
        "ok": true,
        "key": id,
        "deleted": deleted,
    }))
}

pub async fn handle_compact(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: SessionsCompactParams = parse_optional_params("sessions.compact", params)?;
    let max_age_ms = parsed.max_age_ms.unwrap_or(7 * 24 * 60 * 60 * 1_000);
    let removed = state
        .compact_sessions(max_age_ms)
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "ok": true,
        "removed": removed,
        "maxAgeMs": max_age_ms,
    }))
}

fn resolve_session_id(
    id: Option<String>,
    key: Option<String>,
) -> Result<String, crate::protocol::ErrorShape> {
    let candidate = id.or(key).and_then(trim_non_empty);
    let Some(value) = candidate else {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid sessions params: id or key is required",
        ));
    };

    Ok(value)
}

fn sanitize_tags(tags: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for tag in tags {
        let trimmed = tag.trim();
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
