use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    application::state::SharedState,
    rpc::{
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
};

const TALK_CONFIG_KEY: &str = "runtime/talk/config";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TalkModeParams {
    mode: String,
}

pub async fn handle_config(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: serde_json::Map<String, Value> = parse_optional_params("talk.config", params)?;

    Ok(state
        .get_config_entry_value(TALK_CONFIG_KEY)
        .await
        .map_err(map_domain_error)?
        .unwrap_or_else(default_talk_config))
}

pub async fn handle_mode(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: TalkModeParams = parse_required_params("talk.mode", params)?;
    let mode = parsed.mode.trim();
    if mode.is_empty() {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid talk.mode params: mode is required",
        ));
    }

    let mut config = state
        .get_config_entry_value(TALK_CONFIG_KEY)
        .await
        .map_err(map_domain_error)?
        .unwrap_or_else(default_talk_config);

    if let Some(object) = config.as_object_mut() {
        object.insert("mode".to_owned(), Value::String(mode.to_owned()));
    } else {
        config = json!({ "mode": mode });
    }

    let _ = state
        .set_config_entry_value(TALK_CONFIG_KEY, &config)
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "ok": true,
        "mode": mode,
        "config": config,
    }))
}

fn default_talk_config() -> Value {
    json!({
        "mode": "default",
        "allowMentions": true,
        "safety": "standard",
    })
}
