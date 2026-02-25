use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    application::state::SharedState,
    rpc::{
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
};

const TTS_CONFIG_KEY: &str = "runtime/tts/config";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TtsConvertParams {
    text: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    voice: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TtsProviderParams {
    provider: String,
}

pub async fn handle_status(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: Map<String, Value> = parse_optional_params("tts.status", params)?;
    load_tts_config(state).await
}

pub async fn handle_providers(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: Map<String, Value> = parse_optional_params("tts.providers", params)?;
    let config = load_tts_config(state).await?;

    Ok(json!({
        "providers": config
            .get("providers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![Value::String("mock".to_owned())]),
        "activeProvider": config.get("provider").cloned().unwrap_or(Value::String("mock".to_owned())),
    }))
}

pub async fn handle_enable(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: Map<String, Value> = parse_optional_params("tts.enable", params)?;
    set_tts_enabled(state, true).await
}

pub async fn handle_disable(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let _: Map<String, Value> = parse_optional_params("tts.disable", params)?;
    set_tts_enabled(state, false).await
}

pub async fn handle_set_provider(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: TtsProviderParams = parse_required_params("tts.setProvider", params)?;
    let provider = parsed.provider.trim();
    if provider.is_empty() {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid tts.setProvider params: provider is required",
        ));
    }

    let mut config = load_tts_config(state).await?;
    let providers = config
        .get("providers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if !providers
        .iter()
        .filter_map(Value::as_str)
        .any(|value| value == provider)
    {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            format!("unknown tts provider: {provider}"),
        ));
    }

    if let Some(object) = config.as_object_mut() {
        object.insert("provider".to_owned(), Value::String(provider.to_owned()));
    }

    save_tts_config(state, &config).await?;
    Ok(json!({ "ok": true, "provider": provider, "status": config }))
}

pub async fn handle_convert(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: TtsConvertParams = parse_required_params("tts.convert", params)?;
    let text = parsed.text.trim();
    if text.is_empty() {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_INVALID_REQUEST,
            "invalid tts.convert params: text is required",
        ));
    }

    let config = load_tts_config(state).await?;
    let enabled = config
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if !enabled {
        return Err(crate::protocol::ErrorShape::new(
            crate::protocol::ERROR_UNAVAILABLE,
            "tts is disabled",
        ));
    }

    let provider = parsed
        .provider
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        })
        .or_else(|| {
            config
                .get("provider")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "mock".to_owned());

    Ok(json!({
        "ok": true,
        "provider": provider,
        "voice": parsed.voice,
        "audio": {
            "format": "text/plain",
            "durationMs": text.len().saturating_mul(40),
            "content": text,
        }
    }))
}

async fn set_tts_enabled(
    state: &SharedState,
    enabled: bool,
) -> Result<Value, crate::protocol::ErrorShape> {
    let mut config = load_tts_config(state).await?;
    if let Some(object) = config.as_object_mut() {
        object.insert("enabled".to_owned(), Value::Bool(enabled));
    }

    save_tts_config(state, &config).await?;
    Ok(json!({ "ok": true, "enabled": enabled, "status": config }))
}

async fn load_tts_config(state: &SharedState) -> Result<Value, crate::protocol::ErrorShape> {
    let config = state
        .get_config_entry_value(TTS_CONFIG_KEY)
        .await
        .map_err(map_domain_error)?
        .unwrap_or_else(default_tts_config);

    Ok(match config {
        Value::Object(_) => config,
        _ => default_tts_config(),
    })
}

async fn save_tts_config(
    state: &SharedState,
    config: &Value,
) -> Result<(), crate::protocol::ErrorShape> {
    let _ = state
        .set_config_entry_value(TTS_CONFIG_KEY, config)
        .await
        .map_err(map_domain_error)?;
    Ok(())
}

fn default_tts_config() -> Value {
    Value::Object(Map::from_iter([
        ("enabled".to_owned(), Value::Bool(false)),
        ("provider".to_owned(), Value::String("mock".to_owned())),
        (
            "providers".to_owned(),
            Value::Array(vec![Value::String("mock".to_owned())]),
        ),
    ]))
}
