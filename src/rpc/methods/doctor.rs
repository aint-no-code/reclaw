use serde_json::{Value, json};

use crate::{application::state::SharedState, rpc::methods::parse_optional_params};

pub async fn handle_memory_status(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: serde_json::Map<String, Value> = parse_optional_params("doctor.memory.status", params)?;

    Ok(json!({
        "ok": true,
        "runtime": "rust",
        "uptimeMs": state.uptime_ms(),
        "connections": state.connection_count().await,
        "heapBytes": 0,
        "rssBytes": 0,
        "note": "portable memory metrics are unavailable in this build",
    }))
}
