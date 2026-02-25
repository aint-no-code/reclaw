use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    application::state::SharedState,
    domain::models::{ChatMessage, SessionRecord},
    rpc::{SessionContext, dispatcher::map_domain_error, methods::parse_required_params},
    storage::now_unix_ms,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendParams {
    #[serde(default)]
    session_key: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    channel: Option<String>,
}

pub async fn handle_send(
    state: &SharedState,
    session: &SessionContext,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: SendParams = parse_required_params("send", params)?;

    let session_key = parsed
        .session_key
        .or(parsed.session_id)
        .and_then(trim_non_empty)
        .unwrap_or_else(|| "agent:main:main".to_owned());

    let message = parsed
        .message
        .or(parsed.text)
        .and_then(trim_non_empty)
        .ok_or_else(|| {
            crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "invalid send params: message is required",
            )
        })?;

    ensure_session_exists(state, &session_key).await?;

    let ts = now_unix_ms();
    let entry = ChatMessage {
        id: format!("msg-{}", uuid::Uuid::new_v4()),
        role: "assistant".to_owned(),
        text: message,
        status: "final".to_owned(),
        ts,
        metadata: json!({
            "source": "send",
            "channel": parsed.channel,
            "requestedBy": session.client_id,
        }),
    };

    state
        .append_chat_messages(&session_key, std::slice::from_ref(&entry))
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "ok": true,
        "delivered": true,
        "sessionKey": session_key,
        "message": entry,
    }))
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
        metadata: Value::Object(Map::new()),
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
