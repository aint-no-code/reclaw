use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    application::state::SharedState,
    rpc::{
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
    storage::now_unix_ms,
};

const SKILLS_ENTRIES_KEY: &str = "runtime/skills/entries";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SkillConfig {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    env: Map<String, Value>,
    #[serde(default)]
    bins: Vec<String>,
    #[serde(default)]
    installed: bool,
    #[serde(default)]
    install_id: Option<String>,
    #[serde(default)]
    updated_at_ms: Option<u64>,
}

type SkillEntries = BTreeMap<String, SkillConfig>;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillsStatusParams {
    #[serde(default)]
    agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillsInstallParams {
    name: String,
    install_id: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillsUpdateParams {
    skill_key: String,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    env: Option<Map<String, Value>>,
}

pub async fn handle_status(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: SkillsStatusParams = parse_optional_params("skills.status", params)?;
    let entries = load_entries(state).await?;

    let mut skills = Vec::new();
    for (name, config) in &entries {
        let bins = normalized_bins(&config.bins);
        skills.push(json!({
            "name": name,
            "skillKey": name,
            "enabled": config.enabled.unwrap_or(config.installed),
            "installed": config.installed,
            "eligible": config.installed || config.enabled.unwrap_or(false),
            "blockedByAllowlist": false,
            "missingRequirements": [],
            "requiresBins": bins,
            "updatedAtMs": config.updated_at_ms.unwrap_or(0),
        }));
    }

    let eligible = skills
        .iter()
        .filter(|entry| {
            entry
                .get("eligible")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count();

    Ok(json!({
        "agentId": parsed.agent_id,
        "skills": skills,
        "summary": {
            "total": entries.len(),
            "eligible": eligible,
        }
    }))
}

pub async fn handle_bins(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: Map<String, Value> = parse_optional_params("skills.bins", params)?;
    let entries = load_entries(state).await?;

    let mut bins = BTreeSet::new();
    for config in entries.values() {
        for bin in normalized_bins(&config.bins) {
            bins.insert(bin);
        }
    }

    Ok(json!({
        "bins": bins.into_iter().collect::<Vec<_>>(),
    }))
}

pub async fn handle_install(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: SkillsInstallParams = parse_required_params("skills.install", params)?;
    let skill_name = trim_non_empty(parsed.name).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid skills.install params: name is required",
        )
    })?;
    let install_id = trim_non_empty(parsed.install_id).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid skills.install params: installId is required",
        )
    })?;

    let skill_key = normalize_skill_key(&skill_name);
    if skill_key.is_empty() {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid skills.install params: name is not a valid skill key",
        ));
    }

    let mut entries = load_entries(state).await?;
    let existing = entries.get(&skill_key).cloned().unwrap_or_default();
    let now = now_unix_ms();

    let next = SkillConfig {
        enabled: Some(existing.enabled.unwrap_or(true)),
        api_key: existing.api_key,
        env: existing.env,
        bins: if existing.bins.is_empty() {
            infer_skill_bins(&skill_key)
        } else {
            existing.bins
        },
        installed: true,
        install_id: Some(install_id.clone()),
        updated_at_ms: Some(now),
    };

    entries.insert(skill_key.clone(), next);
    save_entries(state, &entries).await?;

    Ok(json!({
        "ok": true,
        "name": skill_key,
        "installId": install_id,
        "timeoutMs": parsed.timeout_ms,
        "installedAtMs": now,
    }))
}

pub async fn handle_update(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: SkillsUpdateParams = parse_required_params("skills.update", params)?;
    let skill_key_raw = trim_non_empty(parsed.skill_key).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid skills.update params: skillKey is required",
        )
    })?;
    let skill_key = normalize_skill_key(&skill_key_raw);
    if skill_key.is_empty() {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid skills.update params: skillKey is not valid",
        ));
    }

    let mut entries = load_entries(state).await?;
    let mut current = entries.get(&skill_key).cloned().unwrap_or_default();

    if let Some(enabled) = parsed.enabled {
        current.enabled = Some(enabled);
    }

    if let Some(api_key) = parsed.api_key {
        current.api_key = trim_non_empty(api_key);
    }

    if let Some(env) = parsed.env {
        let mut next = current.env.clone();
        for (key, value) in env {
            let trimmed_key = key.trim();
            if trimmed_key.is_empty() {
                continue;
            }

            match value {
                Value::String(raw) => {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        next.remove(trimmed_key);
                    } else {
                        next.insert(trimmed_key.to_owned(), Value::String(trimmed.to_owned()));
                    }
                }
                Value::Null => {
                    next.remove(trimmed_key);
                }
                other => {
                    next.insert(trimmed_key.to_owned(), other);
                }
            }
        }
        current.env = next;
    }

    if current.bins.is_empty() {
        current.bins = infer_skill_bins(&skill_key);
    }
    current.updated_at_ms = Some(now_unix_ms());
    entries.insert(skill_key.clone(), current.clone());
    save_entries(state, &entries).await?;

    Ok(json!({
        "ok": true,
        "skillKey": skill_key,
        "config": current,
    }))
}

async fn load_entries(state: &SharedState) -> Result<SkillEntries, crate::protocol::ErrorShape> {
    let Some(raw) = state
        .get_config_entry_value(SKILLS_ENTRIES_KEY)
        .await
        .map_err(map_domain_error)?
    else {
        return Ok(BTreeMap::new());
    };

    serde_json::from_value::<SkillEntries>(raw).map_err(|error| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_UNAVAILABLE,
            format!("failed to decode skills entries: {error}"),
        )
    })
}

async fn save_entries(
    state: &SharedState,
    entries: &SkillEntries,
) -> Result<(), crate::protocol::ErrorShape> {
    let payload = serde_json::to_value(entries).map_err(|error| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_UNAVAILABLE,
            format!("failed to encode skills entries: {error}"),
        )
    })?;

    let _ = state
        .set_config_entry_value(SKILLS_ENTRIES_KEY, &payload)
        .await
        .map_err(map_domain_error)?;
    Ok(())
}

fn infer_skill_bins(skill_key: &str) -> Vec<String> {
    let mut bins = Vec::new();
    if skill_key.contains("python") || skill_key.contains("pip") {
        bins.push("python3".to_owned());
    }
    if skill_key.contains("node") || skill_key.contains("npm") || skill_key.contains("js") {
        bins.push("node".to_owned());
    }
    if skill_key.contains("git") {
        bins.push("git".to_owned());
    }
    if bins.is_empty() {
        bins.push("sh".to_owned());
    }

    normalized_bins(&bins)
}

fn normalized_bins(values: &[String]) -> Vec<String> {
    let mut bins = BTreeSet::new();
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            bins.insert(trimmed.to_owned());
        }
    }
    bins.into_iter().collect()
}

fn normalize_skill_key(input: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;

    for ch in input.chars() {
        let normalized = ch.to_ascii_lowercase();
        if normalized.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            out.push(normalized);
            pending_dash = false;
            continue;
        }

        if normalized == '-' || normalized == '_' || normalized.is_ascii_whitespace() {
            pending_dash = true;
        }
    }

    out.trim_matches('-').to_owned()
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
    use super::normalize_skill_key;

    #[test]
    fn normalize_skill_key_collapses_spacing() {
        assert_eq!(normalize_skill_key("  Rust Lint  "), "rust-lint");
        assert_eq!(normalize_skill_key("!!!"), "");
    }
}
