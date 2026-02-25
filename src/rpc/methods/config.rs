use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    application::state::SharedState,
    rpc::{
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigWriteParams {
    #[serde(default)]
    config: Option<Value>,
    #[serde(default)]
    raw: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigPatchParams {
    #[serde(default)]
    patch: Option<Value>,
    #[serde(default)]
    raw: Option<Value>,
}

pub async fn handle_get(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: Map<String, Value> = parse_optional_params("config.get", params)?;
    state.get_config_doc().await.map_err(map_domain_error)
}

pub async fn handle_set(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ConfigWriteParams = parse_required_params("config.set", params)?;
    let config = resolve_config_value(parsed, "config.set")?;
    state
        .set_config_doc(config.clone())
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "ok": true,
        "path": state.config().db_path.display().to_string(),
        "config": config,
    }))
}

pub async fn handle_apply(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ConfigWriteParams = parse_required_params("config.apply", params)?;
    let config = resolve_config_value(parsed, "config.apply")?;
    state
        .set_config_doc(config.clone())
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "ok": true,
        "path": state.config().db_path.display().to_string(),
        "config": config,
    }))
}

pub async fn handle_patch(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ConfigPatchParams = parse_required_params("config.patch", params)?;
    let patch = resolve_patch_value(parsed)?;

    let mut current = state.get_config_doc().await.map_err(map_domain_error)?;
    merge_patch(&mut current, patch);

    if !current.is_object() {
        current = Value::Object(Map::new());
    }

    state
        .set_config_doc(current.clone())
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "ok": true,
        "path": state.config().db_path.display().to_string(),
        "config": current,
    }))
}

#[must_use]
pub fn handle_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Reclaw Config",
        "type": "object",
        "additionalProperties": true,
        "description": "Runtime configuration document persisted in SQLite.",
    })
}

fn resolve_config_value(
    parsed: ConfigWriteParams,
    method: &str,
) -> Result<Value, crate::protocol::ErrorShape> {
    let Some(config) = parsed.config.or(parsed.raw) else {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            format!("invalid {method} params: config object required"),
        ));
    };

    if !config.is_object() {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            format!("invalid {method} params: config must be an object"),
        ));
    }

    Ok(config)
}

fn resolve_patch_value(parsed: ConfigPatchParams) -> Result<Value, crate::protocol::ErrorShape> {
    let Some(patch) = parsed.patch.or(parsed.raw) else {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid config.patch params: patch object required",
        ));
    };

    if !patch.is_object() {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid config.patch params: patch must be an object",
        ));
    }

    Ok(patch)
}

fn merge_patch(target: &mut Value, patch: Value) {
    let Value::Object(patch_map) = patch else {
        *target = patch;
        return;
    };

    let target_map = match target {
        Value::Object(map) => map,
        _ => {
            *target = Value::Object(Map::new());
            match target {
                Value::Object(map) => map,
                _ => unreachable!(),
            }
        }
    };

    for (key, patch_value) in patch_map {
        if patch_value.is_null() {
            target_map.remove(&key);
            continue;
        }

        if patch_value.is_object() {
            if let Some(existing) = target_map.get_mut(&key) {
                merge_patch(existing, patch_value);
            } else {
                target_map.insert(key, patch_value);
            }
            continue;
        }

        target_map.insert(key, patch_value);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::merge_patch;

    #[test]
    fn merge_patch_removes_null_keys() {
        let mut base = json!({ "a": 1, "b": 2 });
        let patch = json!({ "b": null, "c": 3 });
        merge_patch(&mut base, patch);
        assert_eq!(base, json!({ "a": 1, "c": 3 }));
    }
}
