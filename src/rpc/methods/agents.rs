use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::fs;

use crate::{
    application::state::SharedState,
    rpc::{
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
    storage::now_unix_ms,
};

const AGENTS_REGISTRY_KEY: &str = "runtime/agents/registry";
const DEFAULT_AGENT_ID: &str = "main";

const DEFAULT_AGENTS_FILENAME: &str = "AGENTS.md";
const DEFAULT_SOUL_FILENAME: &str = "SOUL.md";
const DEFAULT_TOOLS_FILENAME: &str = "TOOLS.md";
const DEFAULT_IDENTITY_FILENAME: &str = "IDENTITY.md";
const DEFAULT_USER_FILENAME: &str = "USER.md";
const DEFAULT_HEARTBEAT_FILENAME: &str = "HEARTBEAT.md";
const DEFAULT_BOOTSTRAP_FILENAME: &str = "BOOTSTRAP.md";
const DEFAULT_MEMORY_FILENAME: &str = "MEMORY.md";
const DEFAULT_MEMORY_ALT_FILENAME: &str = "memory.md";

const BOOTSTRAP_FILE_NAMES: &[&str] = &[
    DEFAULT_AGENTS_FILENAME,
    DEFAULT_SOUL_FILENAME,
    DEFAULT_TOOLS_FILENAME,
    DEFAULT_IDENTITY_FILENAME,
    DEFAULT_USER_FILENAME,
    DEFAULT_HEARTBEAT_FILENAME,
    DEFAULT_BOOTSTRAP_FILENAME,
];

const ALLOWED_FILE_NAMES: &[&str] = &[
    DEFAULT_AGENTS_FILENAME,
    DEFAULT_SOUL_FILENAME,
    DEFAULT_TOOLS_FILENAME,
    DEFAULT_IDENTITY_FILENAME,
    DEFAULT_USER_FILENAME,
    DEFAULT_HEARTBEAT_FILENAME,
    DEFAULT_BOOTSTRAP_FILENAME,
    DEFAULT_MEMORY_FILENAME,
    DEFAULT_MEMORY_ALT_FILENAME,
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentRecord {
    agent_id: String,
    name: String,
    workspace: String,
    model: Option<String>,
    avatar: Option<String>,
    created_at_ms: u64,
    updated_at_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentsListParams {
    #[serde(default)]
    include_usage: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentsCreateParams {
    name: String,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    emoji: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentsUpdateParams {
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentsDeleteParams {
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    delete_files: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentsFilesListParams {
    agent_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentsFilesGetParams {
    agent_id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentsFilesSetParams {
    agent_id: String,
    name: String,
    #[serde(default)]
    content: Option<String>,
}

pub async fn handle_list(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: AgentsListParams = parse_optional_params("agents.list", params)?;
    let include_usage = parsed.include_usage.unwrap_or(true);

    let agents = load_agents(state).await?;
    let sessions = if include_usage {
        Some(state.list_sessions().await.map_err(map_domain_error)?)
    } else {
        None
    };

    let mut items = Vec::new();
    for agent in &agents {
        let sessions_count = sessions
            .as_ref()
            .map(|list| {
                list.iter()
                    .filter(|session| {
                        session_agent_id(&session.id).is_some_and(|id| id == agent.agent_id)
                    })
                    .count()
            })
            .unwrap_or(0);

        let workspace_path = PathBuf::from(&agent.workspace);
        let bootstrap_pending = fs::metadata(workspace_path.join(DEFAULT_BOOTSTRAP_FILENAME))
            .await
            .is_ok();

        items.push(json!({
            "id": agent.agent_id,
            "name": agent.name,
            "workspace": agent.workspace,
            "model": agent.model,
            "avatar": agent.avatar,
            "createdAtMs": agent.created_at_ms,
            "updatedAtMs": agent.updated_at_ms,
            "sessionsCount": sessions_count,
            "bootstrapPending": bootstrap_pending,
        }));
    }

    Ok(json!({
        "defaultId": DEFAULT_AGENT_ID,
        "agents": items,
        "count": items.len(),
    }))
}

pub async fn handle_create(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: AgentsCreateParams = parse_required_params("agents.create", params)?;
    let raw_name = trim_non_empty(parsed.name).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid agents.create params: name is required",
        )
    })?;

    let agent_id = normalize_agent_id(&raw_name);
    if agent_id.is_empty() {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid agents.create params: name does not yield a valid agent id",
        ));
    }
    if agent_id == DEFAULT_AGENT_ID {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "\"main\" is reserved",
        ));
    }

    let mut agents = load_agents(state).await?;
    if agents.iter().any(|agent| agent.agent_id == agent_id) {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            format!("agent \"{agent_id}\" already exists"),
        ));
    }

    let workspace_path = resolve_workspace_path(state, parsed.workspace.as_deref(), &agent_id);
    ensure_workspace_bootstrap_files(&workspace_path, &raw_name, parsed.emoji.as_deref())
        .await
        .map_err(storage_error)?;

    let now = now_unix_ms();
    let record = AgentRecord {
        agent_id: agent_id.clone(),
        name: raw_name.clone(),
        workspace: workspace_path.display().to_string(),
        model: parsed.model.and_then(trim_non_empty),
        avatar: parsed.avatar.and_then(trim_non_empty),
        created_at_ms: now,
        updated_at_ms: now,
    };
    agents.push(record.clone());
    save_agents(state, &agents).await?;

    Ok(json!({
        "ok": true,
        "agentId": record.agent_id,
        "name": record.name,
        "workspace": record.workspace,
    }))
}

pub async fn handle_update(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: AgentsUpdateParams = parse_required_params("agents.update", params)?;
    let agent_id = parsed
        .agent_id
        .or(parsed.id)
        .and_then(trim_non_empty)
        .ok_or_else(|| {
            crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "invalid agents.update params: agentId is required",
            )
        })?;

    let mut agents = load_agents(state).await?;
    let Some(index) = agents.iter().position(|agent| agent.agent_id == agent_id) else {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            format!("agent \"{agent_id}\" not found"),
        ));
    };

    let mut next = agents[index].clone();
    if let Some(name) = parsed.name.and_then(trim_non_empty) {
        next.name = name;
    }

    if let Some(workspace) = parsed.workspace.as_deref() {
        let workspace_path = resolve_workspace_path(state, Some(workspace), &agent_id);
        ensure_workspace_bootstrap_files(&workspace_path, &next.name, None)
            .await
            .map_err(storage_error)?;
        next.workspace = workspace_path.display().to_string();
    }

    if let Some(model) = parsed.model {
        next.model = trim_non_empty(model);
    }
    if let Some(avatar) = parsed.avatar {
        next.avatar = trim_non_empty(avatar);
    }
    next.updated_at_ms = now_unix_ms();

    agents[index] = next.clone();
    save_agents(state, &agents).await?;

    Ok(json!({
        "ok": true,
        "agentId": next.agent_id,
    }))
}

pub async fn handle_delete(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: AgentsDeleteParams = parse_required_params("agents.delete", params)?;
    let agent_id = parsed
        .agent_id
        .or(parsed.id)
        .and_then(trim_non_empty)
        .ok_or_else(|| {
            crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "invalid agents.delete params: agentId is required",
            )
        })?;

    if agent_id == DEFAULT_AGENT_ID {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "\"main\" cannot be deleted",
        ));
    }

    let mut agents = load_agents(state).await?;
    let Some(index) = agents.iter().position(|agent| agent.agent_id == agent_id) else {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            format!("agent \"{agent_id}\" not found"),
        ));
    };

    let removed = agents.remove(index);
    save_agents(state, &agents).await?;

    if parsed.delete_files.unwrap_or(true) {
        let workspace_path = PathBuf::from(&removed.workspace);
        if fs::metadata(&workspace_path).await.is_ok() {
            let _ = fs::remove_dir_all(&workspace_path).await;
        }
    }

    Ok(json!({
        "ok": true,
        "agentId": removed.agent_id,
        "removedBindings": 1,
    }))
}

pub async fn handle_files_list(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: AgentsFilesListParams = parse_required_params("agents.files.list", params)?;
    let agent = resolve_agent_by_id(state, &parsed.agent_id).await?;
    let workspace = PathBuf::from(&agent.workspace);
    ensure_workspace_bootstrap_files(&workspace, &agent.name, None)
        .await
        .map_err(storage_error)?;

    let mut files = Vec::new();
    for name in ALLOWED_FILE_NAMES {
        let file_path = workspace.join(name);
        let metadata = fs::metadata(&file_path).await.ok();
        let (missing, size, updated_at_ms) = if let Some(meta) = metadata {
            let updated = meta.modified().ok().and_then(unix_ms).unwrap_or(0);
            (false, Some(meta.len()), Some(updated))
        } else {
            (true, None, None)
        };
        files.push(json!({
            "name": name,
            "path": file_path.display().to_string(),
            "missing": missing,
            "size": size,
            "updatedAtMs": updated_at_ms,
        }));
    }

    Ok(json!({
        "agentId": agent.agent_id,
        "workspace": agent.workspace,
        "files": files,
    }))
}

pub async fn handle_files_get(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: AgentsFilesGetParams = parse_required_params("agents.files.get", params)?;
    let name = validate_agent_file_name("agents.files.get", &parsed.name)?;
    let agent = resolve_agent_by_id(state, &parsed.agent_id).await?;
    let workspace = PathBuf::from(&agent.workspace);
    ensure_workspace_bootstrap_files(&workspace, &agent.name, None)
        .await
        .map_err(storage_error)?;

    let path = workspace.join(&name);
    let metadata = fs::metadata(&path).await.ok();
    if metadata.is_none() {
        return Ok(json!({
            "agentId": agent.agent_id,
            "workspace": agent.workspace,
            "file": {
                "name": name,
                "path": path.display().to_string(),
                "missing": true,
            }
        }));
    }

    let content = fs::read_to_string(&path).await.map_err(storage_error)?;
    let updated_at_ms = fs::metadata(&path)
        .await
        .ok()
        .and_then(|meta| meta.modified().ok().and_then(unix_ms))
        .unwrap_or(0);
    let size = u64::try_from(content.len()).unwrap_or(u64::MAX);

    Ok(json!({
        "agentId": agent.agent_id,
        "workspace": agent.workspace,
        "file": {
            "name": name,
            "path": path.display().to_string(),
            "missing": false,
            "size": size,
            "updatedAtMs": updated_at_ms,
            "content": content,
        }
    }))
}

pub async fn handle_files_set(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: AgentsFilesSetParams = parse_required_params("agents.files.set", params)?;
    let name = validate_agent_file_name("agents.files.set", &parsed.name)?;
    let content = parsed.content.unwrap_or_default();
    let agent = resolve_agent_by_id(state, &parsed.agent_id).await?;
    let workspace = PathBuf::from(&agent.workspace);
    fs::create_dir_all(&workspace)
        .await
        .map_err(storage_error)?;

    let path = workspace.join(&name);
    fs::write(&path, &content).await.map_err(storage_error)?;
    let updated_at_ms = fs::metadata(&path)
        .await
        .ok()
        .and_then(|meta| meta.modified().ok().and_then(unix_ms))
        .unwrap_or(now_unix_ms());
    let size = u64::try_from(content.len()).unwrap_or(u64::MAX);

    Ok(json!({
        "ok": true,
        "agentId": agent.agent_id,
        "workspace": agent.workspace,
        "file": {
            "name": name,
            "path": path.display().to_string(),
            "missing": false,
            "size": size,
            "updatedAtMs": updated_at_ms,
            "content": content,
        }
    }))
}

async fn resolve_agent_by_id(
    state: &SharedState,
    agent_id_raw: &str,
) -> Result<AgentRecord, crate::protocol::ErrorShape> {
    let agent_id = trim_non_empty(agent_id_raw.to_owned()).ok_or_else(|| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid params: agentId is required",
        )
    })?;

    let agents = load_agents(state).await?;
    agents
        .into_iter()
        .find(|agent| agent.agent_id == agent_id)
        .ok_or_else(|| {
            crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                format!("agent \"{agent_id}\" not found"),
            )
        })
}

async fn load_agents(state: &SharedState) -> Result<Vec<AgentRecord>, crate::protocol::ErrorShape> {
    let Some(raw) = state
        .get_config_entry_value(AGENTS_REGISTRY_KEY)
        .await
        .map_err(map_domain_error)?
    else {
        return Ok(vec![default_main_agent(state)]);
    };

    match serde_json::from_value::<Vec<AgentRecord>>(raw) {
        Ok(entries) if !entries.is_empty() => Ok(entries),
        Ok(_) => Ok(vec![default_main_agent(state)]),
        Err(error) => Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_UNAVAILABLE,
            format!("failed to decode agents registry: {error}"),
        )),
    }
}

async fn save_agents(
    state: &SharedState,
    agents: &[AgentRecord],
) -> Result<(), crate::protocol::ErrorShape> {
    let value = serde_json::to_value(agents).map_err(|error| {
        crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_UNAVAILABLE,
            format!("failed to serialize agents registry: {error}"),
        )
    })?;

    let _ = state
        .set_config_entry_value(AGENTS_REGISTRY_KEY, &value)
        .await
        .map_err(map_domain_error)?;
    Ok(())
}

fn default_main_agent(state: &SharedState) -> AgentRecord {
    let now = now_unix_ms();
    let workspace = resolve_workspace_path(state, None, DEFAULT_AGENT_ID);

    AgentRecord {
        agent_id: DEFAULT_AGENT_ID.to_owned(),
        name: "Main".to_owned(),
        workspace: workspace.display().to_string(),
        model: None,
        avatar: None,
        created_at_ms: now,
        updated_at_ms: now,
    }
}

fn resolve_workspace_path(state: &SharedState, raw: Option<&str>, agent_id: &str) -> PathBuf {
    if let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) {
        return expand_home(raw);
    }

    agents_root(state).join(agent_id)
}

fn agents_root(state: &SharedState) -> PathBuf {
    state
        .config()
        .db_path
        .parent()
        .map_or_else(|| PathBuf::from("./agents"), |path| path.join("agents"))
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home);
    }

    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }

    PathBuf::from(path)
}

async fn ensure_workspace_bootstrap_files(
    workspace: &Path,
    agent_name: &str,
    emoji: Option<&str>,
) -> Result<(), std::io::Error> {
    fs::create_dir_all(workspace).await?;

    for name in BOOTSTRAP_FILE_NAMES {
        let path = workspace.join(name);
        if fs::metadata(&path).await.is_ok() {
            continue;
        }

        let template = bootstrap_file_template(name, agent_name, emoji);
        fs::write(path, template).await?;
    }

    let memory = workspace.join(DEFAULT_MEMORY_FILENAME);
    if fs::metadata(&memory).await.is_err() {
        fs::write(memory, "# Memory\n\n").await?;
    }

    Ok(())
}

fn bootstrap_file_template(name: &str, agent_name: &str, emoji: Option<&str>) -> String {
    match name {
        DEFAULT_IDENTITY_FILENAME => {
            let emoji_line = emoji
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map_or_else(String::new, |value| format!("- Emoji: {value}\n"));

            format!("# Identity\n\n- Name: {agent_name}\n{emoji_line}")
        }
        DEFAULT_AGENTS_FILENAME => "# Agents\n\n".to_owned(),
        DEFAULT_SOUL_FILENAME => "# Soul\n\n".to_owned(),
        DEFAULT_TOOLS_FILENAME => "# Tools\n\n".to_owned(),
        DEFAULT_USER_FILENAME => "# User\n\n".to_owned(),
        DEFAULT_HEARTBEAT_FILENAME => "# Heartbeat\n\n".to_owned(),
        DEFAULT_BOOTSTRAP_FILENAME => "# Bootstrap\n\n".to_owned(),
        _ => String::new(),
    }
}

fn validate_agent_file_name(
    method: &str,
    name_raw: &str,
) -> Result<String, crate::protocol::ErrorShape> {
    let name = name_raw.trim();
    if ALLOWED_FILE_NAMES.contains(&name) {
        return Ok(name.to_owned());
    }

    Err(crate::protocol::ErrorShape::new(
        crate::protocol::ERROR_INVALID_REQUEST,
        format!("invalid {method} params: unsupported file \"{name}\""),
    ))
}

fn session_agent_id(session_id: &str) -> Option<&str> {
    let mut parts = session_id.split(':');
    let prefix = parts.next()?;
    if prefix != "agent" {
        return None;
    }

    parts.next().filter(|value| !value.trim().is_empty())
}

fn unix_ms(system_time: std::time::SystemTime) -> Option<u64> {
    let duration = system_time.duration_since(std::time::UNIX_EPOCH).ok()?;
    u64::try_from(duration.as_millis()).ok()
}

fn storage_error(error: std::io::Error) -> crate::protocol::ErrorShape {
    crate::protocol::ErrorShape::new(
        crate::protocol::ERROR_UNAVAILABLE,
        format!("storage error: {error}"),
    )
}

fn normalize_agent_id(name: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;

    for ch in name.chars() {
        let normalized = ch.to_ascii_lowercase();
        let keep = normalized.is_ascii_alphanumeric();
        if keep {
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
    use super::normalize_agent_id;

    #[test]
    fn normalize_agent_id_strips_invalid_characters() {
        assert_eq!(normalize_agent_id("Team Alpha 🤖"), "team-alpha");
        assert_eq!(normalize_agent_id("___Main___"), "main");
    }
}
