use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    application::state::SharedState,
    rpc::{
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
    storage::now_unix_ms,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageCostParams {
    #[serde(default)]
    period_days: Option<u32>,
}

pub async fn handle_status(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: serde_json::Map<String, Value> = parse_optional_params("usage.status", params)?;

    let sessions = state.list_sessions().await.map_err(map_domain_error)?;
    let nodes = state.list_nodes().await.map_err(map_domain_error)?;
    let cron_jobs = state.list_cron_jobs().await.map_err(map_domain_error)?;
    let chat_messages = state
        .count_chat_messages()
        .await
        .map_err(map_domain_error)?;
    let agent_runs = state.count_agent_runs().await.map_err(map_domain_error)?;
    let log_entries = state
        .list_config_entries("logs/", Some(5_000))
        .await
        .map_err(map_domain_error)?
        .len();

    Ok(json!({
        "ts": now_unix_ms(),
        "runtime": "reclaw-core",
        "counts": {
            "sessions": sessions.len(),
            "nodes": nodes.len(),
            "cronJobs": cron_jobs.len(),
            "chatMessages": chat_messages,
            "agentRuns": agent_runs,
            "logEntries": log_entries,
        }
    }))
}

pub async fn handle_cost(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: UsageCostParams = parse_required_params("usage.cost", params)?;
    let period_days = parsed.period_days.unwrap_or(30).clamp(1, 365);

    let chat_messages = state
        .count_chat_messages()
        .await
        .map_err(map_domain_error)? as f64;
    let agent_runs = state.count_agent_runs().await.map_err(map_domain_error)? as f64;

    let estimated_tokens = (chat_messages * 350.0) + (agent_runs * 500.0);
    let estimated_cost_usd = (estimated_tokens / 1_000.0) * 0.0025;

    Ok(json!({
        "periodDays": period_days,
        "currency": "USD",
        "estimatedTokens": estimated_tokens.round() as u64,
        "estimatedCostUsd": (estimated_cost_usd * 10_000.0).round() / 10_000.0,
        "assumptions": {
            "avgChatTokens": 350,
            "avgAgentTokens": 500,
            "pricePer1kTokensUsd": 0.0025,
        }
    }))
}
