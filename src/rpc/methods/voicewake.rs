use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    application::state::SharedState,
    rpc::{
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
};

const VOICEWAKE_CONFIG_KEY: &str = "runtime/voicewake/config";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VoicewakeSetParams {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    phrase: Option<String>,
}

pub async fn handle_get(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: serde_json::Map<String, Value> = parse_optional_params("voicewake.get", params)?;

    Ok(state
        .get_config_entry_value(VOICEWAKE_CONFIG_KEY)
        .await
        .map_err(map_domain_error)?
        .unwrap_or_else(default_voicewake_config))
}

pub async fn handle_set(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: VoicewakeSetParams = parse_required_params("voicewake.set", params)?;

    let mut config = state
        .get_config_entry_value(VOICEWAKE_CONFIG_KEY)
        .await
        .map_err(map_domain_error)?
        .unwrap_or_else(default_voicewake_config);

    let Some(object) = config.as_object_mut() else {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "voicewake config is corrupted",
        ));
    };

    if let Some(enabled) = parsed.enabled {
        object.insert("enabled".to_owned(), Value::Bool(enabled));
    }

    if let Some(phrase) = parsed.phrase {
        let trimmed = phrase.trim();
        if trimmed.is_empty() {
            return Err(crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "invalid voicewake.set params: phrase cannot be empty",
            ));
        }
        object.insert("phrase".to_owned(), Value::String(trimmed.to_owned()));
    }

    let _ = state
        .set_config_entry_value(VOICEWAKE_CONFIG_KEY, &config)
        .await
        .map_err(map_domain_error)?;

    Ok(json!({ "ok": true, "config": config }))
}

fn default_voicewake_config() -> Value {
    json!({
        "enabled": false,
        "phrase": "hey reclaw",
    })
}
