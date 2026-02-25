use serde_json::{Value, json};

use crate::{application::state::SharedState, rpc::methods::parse_optional_params};

pub fn handle_catalog(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: serde_json::Map<String, Value> = parse_optional_params("tools.catalog", params)?;

    let tools = vec![
        json!({
            "id": "gateway.request",
            "kind": "rpc",
            "description": "Invoke gateway RPC method",
        }),
        json!({
            "id": "sessions",
            "kind": "state",
            "description": "List and mutate sessions",
        }),
        json!({
            "id": "cron",
            "kind": "scheduler",
            "description": "Create and run scheduled jobs",
        }),
        json!({
            "id": "nodes",
            "kind": "device",
            "description": "Pair and invoke registered nodes",
        }),
    ];

    Ok(json!({
        "runtime": "reclaw-core",
        "methods": state.methods(),
        "tools": tools,
    }))
}
