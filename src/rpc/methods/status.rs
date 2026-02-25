use serde_json::{Value, json};

use crate::{application::state::SharedState, rpc::SessionContext};

pub async fn handle(state: &SharedState, session: &SessionContext) -> Value {
    json!({
        "ok": true,
        "runtime": "rust",
        "version": state.config().runtime_version,
        "authMode": state.auth_mode_label(),
        "uptimeMs": state.uptime_ms(),
        "connections": state.connection_count().await,
        "session": {
            "connId": session.conn_id,
            "role": session.role,
            "scopes": session.scopes,
            "clientId": session.client_id,
            "clientMode": session.client_mode,
        }
    })
}

#[must_use]
pub fn info_payload(state: &SharedState) -> Value {
    json!({
        "name": "reclaw-core",
        "runtime": "rust",
        "version": state.config().runtime_version,
        "lineage": {
            "forkedFrom": "openclaw",
            "upstream": "https://github.com/openclaw/openclaw"
        },
        "protocolVersion": crate::protocol::PROTOCOL_VERSION,
        "authMode": state.auth_mode_label(),
        "methods": state.methods(),
        "events": state.events(),
    })
}
