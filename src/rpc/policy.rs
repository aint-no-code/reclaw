use crate::protocol::{ERROR_INVALID_REQUEST, ErrorShape};

use super::SessionContext;

pub const ADMIN_SCOPE: &str = "operator.admin";
pub const READ_SCOPE: &str = "operator.read";
pub const WRITE_SCOPE: &str = "operator.write";
pub const APPROVALS_SCOPE: &str = "operator.approvals";
pub const PAIRING_SCOPE: &str = "operator.pairing";

const NODE_ROLE_METHODS: &[&str] = &["node.invoke.result", "node.event", "skills.bins"];
const CONTROL_PLANE_WRITE_METHODS: &[&str] = &["config.apply", "config.patch", "update.run"];

#[must_use]
pub fn is_control_plane_write_method(method: &str) -> bool {
    CONTROL_PLANE_WRITE_METHODS.contains(&method)
}

#[must_use]
pub fn default_operator_scopes() -> Vec<String> {
    vec![
        ADMIN_SCOPE.to_owned(),
        READ_SCOPE.to_owned(),
        WRITE_SCOPE.to_owned(),
        APPROVALS_SCOPE.to_owned(),
        PAIRING_SCOPE.to_owned(),
    ]
}

pub fn authorize_session(session: &SessionContext, method: &str) -> Result<(), ErrorShape> {
    if method == "health" {
        return Ok(());
    }

    let role = session.role.as_str();
    if role != "operator" && role != "node" {
        return Err(ErrorShape::new(
            ERROR_INVALID_REQUEST,
            format!("unauthorized role: {role}"),
        ));
    }

    if NODE_ROLE_METHODS.contains(&method) {
        if role != "node" {
            return Err(ErrorShape::new(
                ERROR_INVALID_REQUEST,
                format!("unauthorized role: {role}"),
            ));
        }
        return Ok(());
    }

    if role != "operator" {
        return Err(ErrorShape::new(
            ERROR_INVALID_REQUEST,
            format!("unauthorized role: {role}"),
        ));
    }

    if session.scopes.iter().any(|scope| scope == ADMIN_SCOPE) {
        return Ok(());
    }

    let required = required_scope_for_method(method).unwrap_or(ADMIN_SCOPE);

    if required == READ_SCOPE {
        if session
            .scopes
            .iter()
            .any(|scope| scope == READ_SCOPE || scope == WRITE_SCOPE)
        {
            return Ok(());
        }
        return Err(ErrorShape::new(
            ERROR_INVALID_REQUEST,
            format!("missing scope: {READ_SCOPE}"),
        ));
    }

    if session.scopes.iter().any(|scope| scope == required) {
        return Ok(());
    }

    Err(ErrorShape::new(
        ERROR_INVALID_REQUEST,
        format!("missing scope: {required}"),
    ))
}

fn required_scope_for_method(method: &str) -> Option<&'static str> {
    match method {
        "exec.approval.request" | "exec.approval.waitDecision" | "exec.approval.resolve" => {
            Some(APPROVALS_SCOPE)
        }
        "node.pair.request"
        | "node.pair.list"
        | "node.pair.approve"
        | "node.pair.reject"
        | "node.pair.verify"
        | "device.pair.list"
        | "device.pair.approve"
        | "device.pair.reject"
        | "device.pair.remove"
        | "device.token.rotate"
        | "device.token.revoke"
        | "node.rename" => Some(PAIRING_SCOPE),
        "health"
        | "doctor.memory.status"
        | "logs.tail"
        | "channels.status"
        | "status"
        | "usage.status"
        | "usage.cost"
        | "tts.status"
        | "tts.providers"
        | "models.list"
        | "tools.catalog"
        | "agents.list"
        | "agent.identity.get"
        | "skills.status"
        | "voicewake.get"
        | "sessions.list"
        | "sessions.preview"
        | "cron.list"
        | "cron.status"
        | "cron.runs"
        | "system-presence"
        | "last-heartbeat"
        | "node.list"
        | "node.describe"
        | "chat.history"
        | "config.get"
        | "talk.config"
        | "agents.files.list"
        | "agents.files.get" => Some(READ_SCOPE),
        "send" | "agent" | "agent.wait" | "wake" | "talk.mode" | "tts.enable" | "tts.disable"
        | "tts.convert" | "tts.setProvider" | "voicewake.set" | "node.invoke" | "chat.send"
        | "chat.abort" | "browser.request" => Some(WRITE_SCOPE),
        "channels.logout" | "agents.create" | "agents.update" | "agents.delete"
        | "skills.install" | "skills.update" | "cron.add" | "cron.update" | "cron.remove"
        | "cron.run" | "sessions.patch" | "sessions.reset" | "sessions.delete"
        | "sessions.compact" | "connect" | "set-heartbeats" | "system-event"
        | "agents.files.set" => Some(ADMIN_SCOPE),
        _ => {
            if method.starts_with("exec.approvals.")
                || method.starts_with("config.")
                || method.starts_with("wizard.")
                || method.starts_with("update.")
            {
                Some(ADMIN_SCOPE)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::rpc::SessionContext;

    use super::{authorize_session, default_operator_scopes};

    #[test]
    fn operator_defaults_can_call_admin_method() {
        let session = SessionContext {
            conn_id: "c1".to_owned(),
            role: "operator".to_owned(),
            scopes: default_operator_scopes(),
            client_id: "cli".to_owned(),
            client_mode: "cli".to_owned(),
        };

        assert!(authorize_session(&session, "wizard.start").is_ok());
    }

    #[test]
    fn node_role_is_restricted_from_operator_methods() {
        let session = SessionContext {
            conn_id: "c1".to_owned(),
            role: "node".to_owned(),
            scopes: Vec::new(),
            client_id: "node-a".to_owned(),
            client_mode: "node".to_owned(),
        };

        assert!(authorize_session(&session, "chat.send").is_err());
        assert!(authorize_session(&session, "node.event").is_ok());
    }
}
