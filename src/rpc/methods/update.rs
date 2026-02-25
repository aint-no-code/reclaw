use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    application::state::SharedState,
    rpc::{SessionContext, dispatcher::map_domain_error, methods::parse_optional_params},
    storage::now_unix_ms,
};

const UPDATE_LAST_RUN_KEY: &str = "runtime/update/last-run";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRunParams {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

pub async fn handle_run(
    state: &SharedState,
    session: &SessionContext,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: UpdateRunParams = parse_optional_params("update.run", params)?;
    let payload = json!({
        "id": format!("update-{}", uuid::Uuid::new_v4()),
        "mode": parsed.mode.and_then(trim_non_empty).unwrap_or_else(|| "check".to_owned()),
        "note": parsed.note.and_then(trim_non_empty),
        "ts": now_unix_ms(),
        "requestedBy": session.client_id,
        "status": "completed",
        "restartRequired": false,
    });

    let _ = state
        .set_config_entry_value(UPDATE_LAST_RUN_KEY, &payload)
        .await
        .map_err(map_domain_error)?;

    Ok(payload)
}

fn trim_non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
