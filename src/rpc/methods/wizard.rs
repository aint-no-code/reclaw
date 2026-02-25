use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    application::state::SharedState,
    rpc::{
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
    storage::now_unix_ms,
};

const WIZARD_PREFIX: &str = "wizard/session/";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WizardSession {
    id: String,
    status: String,
    goal: String,
    step_index: usize,
    steps: Vec<String>,
    created_at_ms: u64,
    updated_at_ms: u64,
    last_input: Option<String>,
    cancel_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WizardStartParams {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    goal: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WizardNextParams {
    id: String,
    #[serde(default)]
    input: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WizardCancelParams {
    id: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WizardStatusParams {
    id: String,
}

pub async fn handle_start(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: WizardStartParams = parse_required_params("wizard.start", params)?;
    let goal = parsed
        .goal
        .or(parsed.prompt)
        .and_then(trim_non_empty)
        .ok_or_else(|| {
            crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "invalid wizard.start params: goal is required",
            )
        })?;

    let now = now_unix_ms();
    let id = parsed
        .id
        .and_then(trim_non_empty)
        .unwrap_or_else(|| format!("wizard-{}", uuid::Uuid::new_v4()));

    let session = WizardSession {
        id: id.clone(),
        status: "active".to_owned(),
        goal,
        step_index: 0,
        steps: vec![
            "collect-requirements".to_owned(),
            "validate-plan".to_owned(),
            "apply-changes".to_owned(),
            "verify-results".to_owned(),
        ],
        created_at_ms: now,
        updated_at_ms: now,
        last_input: None,
        cancel_reason: None,
    };

    persist_wizard(state, &session).await?;
    Ok(wizard_response(&session))
}

pub async fn handle_next(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: WizardNextParams = parse_required_params("wizard.next", params)?;
    let id = trim_non_empty(parsed.id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid wizard.next params: id is required",
        )
    })?;

    let mut session = load_wizard(state, &id).await?;
    if session.status != "active" {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            format!("wizard session is not active: {}", session.status),
        ));
    }

    if session.step_index + 1 >= session.steps.len() {
        session.status = "completed".to_owned();
    } else {
        session.step_index += 1;
    }

    session.last_input = parsed.input.and_then(trim_non_empty);
    session.updated_at_ms = now_unix_ms();
    persist_wizard(state, &session).await?;

    Ok(wizard_response(&session))
}

pub async fn handle_cancel(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: WizardCancelParams = parse_required_params("wizard.cancel", params)?;
    let id = trim_non_empty(parsed.id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid wizard.cancel params: id is required",
        )
    })?;

    let mut session = load_wizard(state, &id).await?;
    session.status = "cancelled".to_owned();
    session.cancel_reason = parsed.reason.and_then(trim_non_empty);
    session.updated_at_ms = now_unix_ms();
    persist_wizard(state, &session).await?;

    Ok(wizard_response(&session))
}

pub async fn handle_status(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: WizardStatusParams = parse_optional_params("wizard.status", params)?;
    let id = trim_non_empty(parsed.id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid wizard.status params: id is required",
        )
    })?;

    let session = load_wizard(state, &id).await?;
    Ok(wizard_response(&session))
}

async fn persist_wizard(
    state: &SharedState,
    session: &WizardSession,
) -> Result<(), crate::protocol::ErrorShape> {
    let value = serde_json::to_value(session).map_err(|error| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_UNAVAILABLE,
            format!("failed to serialize wizard session: {error}"),
        )
    })?;

    let key = format!("{WIZARD_PREFIX}{}", session.id);
    let _ = state
        .set_config_entry_value(&key, &value)
        .await
        .map_err(map_domain_error)?;
    Ok(())
}

async fn load_wizard(
    state: &SharedState,
    id: &str,
) -> Result<WizardSession, crate::protocol::ErrorShape> {
    let key = format!("{WIZARD_PREFIX}{id}");
    let raw = state
        .get_config_entry_value(&key)
        .await
        .map_err(map_domain_error)?
        .ok_or_else(|| {
            crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                format!("wizard session not found: {id}"),
            )
        })?;

    serde_json::from_value::<WizardSession>(raw).map_err(|error| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_UNAVAILABLE,
            format!("failed to decode wizard session: {error}"),
        )
    })
}

fn wizard_response(session: &WizardSession) -> Value {
    let current_step = session
        .steps
        .get(session.step_index)
        .cloned()
        .unwrap_or_else(|| "done".to_owned());

    json!({
        "id": session.id,
        "status": session.status,
        "goal": session.goal,
        "stepIndex": session.step_index,
        "currentStep": current_step,
        "steps": session.steps,
        "updatedAtMs": session.updated_at_ms,
        "cancelReason": session.cancel_reason,
    })
}

fn trim_non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::wizard_response;

    #[test]
    fn wizard_response_returns_current_step() {
        let session = super::WizardSession {
            id: "w1".to_owned(),
            status: "active".to_owned(),
            goal: "test".to_owned(),
            step_index: 1,
            steps: vec!["a".to_owned(), "b".to_owned()],
            created_at_ms: 1,
            updated_at_ms: 2,
            last_input: None,
            cancel_reason: None,
        };

        let payload = wizard_response(&session);
        assert_eq!(payload["currentStep"], "b");
    }
}
