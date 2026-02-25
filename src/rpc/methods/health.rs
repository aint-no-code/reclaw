use serde_json::{Value, json};

use crate::{
    application::state::SharedState,
    protocol::{ERROR_UNAVAILABLE, ErrorShape},
};

pub async fn handle(state: &SharedState, _params: Option<&Value>) -> Value {
    match state.health_payload().await {
        Ok(payload) => payload,
        Err(error) => json!({
            "ok": false,
            "error": ErrorShape::new(ERROR_UNAVAILABLE, error.to_string()),
        }),
    }
}

#[must_use]
pub fn ready_payload(state: &SharedState, connections: usize) -> Value {
    json!({
        "ok": true,
        "runtime": "rust",
        "version": state.config().runtime_version,
        "connections": connections,
    })
}
