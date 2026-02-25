pub mod agent;
pub mod chat;
pub mod config;
pub mod cron;
pub mod health;
pub mod nodes;
pub mod sessions;
pub mod status;

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::protocol::{ERROR_INVALID_REQUEST, ErrorShape};

pub const BASE_METHODS: &[&str] = &[
    "health",
    "doctor.memory.status",
    "logs.tail",
    "channels.status",
    "channels.logout",
    "status",
    "usage.status",
    "usage.cost",
    "tts.status",
    "tts.providers",
    "tts.enable",
    "tts.disable",
    "tts.convert",
    "tts.setProvider",
    "config.get",
    "config.set",
    "config.apply",
    "config.patch",
    "config.schema",
    "exec.approvals.get",
    "exec.approvals.set",
    "exec.approvals.node.get",
    "exec.approvals.node.set",
    "exec.approval.request",
    "exec.approval.waitDecision",
    "exec.approval.resolve",
    "wizard.start",
    "wizard.next",
    "wizard.cancel",
    "wizard.status",
    "talk.config",
    "talk.mode",
    "models.list",
    "tools.catalog",
    "agents.list",
    "agents.create",
    "agents.update",
    "agents.delete",
    "agents.files.list",
    "agents.files.get",
    "agents.files.set",
    "skills.status",
    "skills.bins",
    "skills.install",
    "skills.update",
    "update.run",
    "voicewake.get",
    "voicewake.set",
    "sessions.list",
    "sessions.preview",
    "sessions.patch",
    "sessions.reset",
    "sessions.delete",
    "sessions.compact",
    "last-heartbeat",
    "set-heartbeats",
    "wake",
    "node.pair.request",
    "node.pair.list",
    "node.pair.approve",
    "node.pair.reject",
    "node.pair.verify",
    "device.pair.list",
    "device.pair.approve",
    "device.pair.reject",
    "device.pair.remove",
    "device.token.rotate",
    "device.token.revoke",
    "node.rename",
    "node.list",
    "node.describe",
    "node.invoke",
    "node.invoke.result",
    "node.event",
    "cron.list",
    "cron.status",
    "cron.add",
    "cron.update",
    "cron.remove",
    "cron.run",
    "cron.runs",
    "system-presence",
    "system-event",
    "send",
    "agent",
    "agent.identity.get",
    "agent.wait",
    "browser.request",
    "chat.history",
    "chat.abort",
    "chat.send",
];

pub const GATEWAY_EVENTS: &[&str] = &[
    "connect.challenge",
    "agent",
    "chat",
    "presence",
    "tick",
    "talk.mode",
    "shutdown",
    "health",
    "heartbeat",
    "cron",
    "node.pair.requested",
    "node.pair.resolved",
    "node.invoke.request",
    "device.pair.requested",
    "device.pair.resolved",
    "voicewake.changed",
    "exec.approval.requested",
    "exec.approval.resolved",
    "update.available",
];

const IMPLEMENTED_METHODS: &[&str] = &[
    "health",
    "status",
    "config.get",
    "config.set",
    "config.apply",
    "config.patch",
    "config.schema",
    "sessions.list",
    "sessions.preview",
    "sessions.patch",
    "sessions.reset",
    "sessions.delete",
    "sessions.compact",
    "agent",
    "agent.identity.get",
    "agent.wait",
    "chat.history",
    "chat.abort",
    "chat.send",
    "cron.list",
    "cron.status",
    "cron.add",
    "cron.update",
    "cron.remove",
    "cron.run",
    "cron.runs",
    "node.pair.request",
    "node.pair.list",
    "node.pair.approve",
    "node.pair.reject",
    "node.pair.verify",
    "node.rename",
    "node.list",
    "node.describe",
    "node.invoke",
    "node.invoke.result",
    "node.event",
];

#[must_use]
pub fn known_methods() -> Vec<String> {
    BASE_METHODS
        .iter()
        .map(|value| (*value).to_owned())
        .collect()
}

#[must_use]
pub fn known_events() -> Vec<String> {
    GATEWAY_EVENTS
        .iter()
        .map(|value| (*value).to_owned())
        .collect()
}

#[must_use]
pub fn implemented_methods() -> Vec<String> {
    IMPLEMENTED_METHODS
        .iter()
        .map(|value| (*value).to_owned())
        .collect()
}

#[must_use]
pub fn is_known_method(method: &str) -> bool {
    BASE_METHODS.contains(&method)
}

#[must_use]
pub fn is_implemented_method(method: &str) -> bool {
    IMPLEMENTED_METHODS.contains(&method)
}

pub(crate) fn parse_optional_params<T: DeserializeOwned>(
    method: &str,
    params: Option<&Value>,
) -> Result<T, ErrorShape> {
    let raw = params.cloned().unwrap_or_else(|| Value::Object(Map::new()));
    serde_json::from_value::<T>(raw).map_err(|error| {
        ErrorShape::new(
            ERROR_INVALID_REQUEST,
            format!("invalid {method} params: {error}"),
        )
    })
}

pub(crate) fn parse_required_params<T: DeserializeOwned>(
    method: &str,
    params: Option<&Value>,
) -> Result<T, ErrorShape> {
    let Some(raw) = params.cloned() else {
        return Err(ErrorShape::new(
            ERROR_INVALID_REQUEST,
            format!("invalid {method} params: object required"),
        ));
    };

    if !raw.is_object() {
        return Err(ErrorShape::new(
            ERROR_INVALID_REQUEST,
            format!("invalid {method} params: object required"),
        ));
    }

    serde_json::from_value::<T>(raw).map_err(|error| {
        ErrorShape::new(
            ERROR_INVALID_REQUEST,
            format!("invalid {method} params: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{implemented_methods, is_implemented_method, is_known_method};

    #[test]
    fn methods_have_known_coverage() {
        assert!(is_known_method("health"));
        assert!(is_implemented_method("health"));
        assert!(is_known_method("wizard.start"));
        assert!(!is_implemented_method("wizard.start"));
        assert!(implemented_methods().len() > 10);
    }
}
