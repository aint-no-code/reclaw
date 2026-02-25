use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    application::state::SharedState,
    rpc::{dispatcher::map_domain_error, methods::parse_optional_params},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogsTailParams {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    method: Option<String>,
}

pub async fn handle_tail(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: LogsTailParams = parse_optional_params("logs.tail", params)?;

    let limit = parsed.limit.unwrap_or(200).clamp(1, 2_000);
    let level_filter = parsed.level.and_then(normalize_string);
    let method_filter = parsed.method.and_then(normalize_string);

    let entries = state
        .list_config_entries("logs/", Some(limit.saturating_mul(4)))
        .await
        .map_err(map_domain_error)?;

    let mut out = Vec::new();
    for entry in entries {
        let value = entry.value;
        if !matches_level(&value, level_filter.as_deref()) {
            continue;
        }
        if !matches_method(&value, method_filter.as_deref()) {
            continue;
        }

        out.push(value);
        if out.len() >= limit {
            break;
        }
    }

    Ok(json!({
        "entries": out,
        "count": out.len(),
    }))
}

fn matches_level(value: &Value, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return true;
    };

    value
        .get("level")
        .and_then(Value::as_str)
        .is_some_and(|level| level.eq_ignore_ascii_case(expected))
}

fn matches_method(value: &Value, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return true;
    };

    value
        .get("method")
        .and_then(Value::as_str)
        .is_some_and(|method| method == expected)
}

fn normalize_string(input: String) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
