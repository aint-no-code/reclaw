use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    application::state::SharedState,
    rpc::{dispatcher::map_domain_error, methods::parse_optional_params},
    storage::now_unix_ms,
};

const MODELS_CATALOG_KEY: &str = "runtime/models/catalog";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelsListParams {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

pub async fn handle_list(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ModelsListParams = parse_optional_params("models.list", params)?;

    let provider_filter = parsed.provider.map(|value| value.trim().to_owned());
    let limit = parsed.limit.unwrap_or(64).clamp(1, 512);

    let mut models = state
        .get_config_entry_value(MODELS_CATALOG_KEY)
        .await
        .map_err(map_domain_error)?
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_else(default_models);

    if let Some(provider) = provider_filter
        && !provider.is_empty()
    {
        models.retain(|item| {
            item.get("provider")
                .and_then(Value::as_str)
                .is_some_and(|current| current == provider)
        });
    }

    if models.len() > limit {
        models.truncate(limit);
    }

    Ok(json!({
        "ts": now_unix_ms(),
        "models": models,
    }))
}

fn default_models() -> Vec<Value> {
    vec![
        json!({
            "id": "gpt-5",
            "provider": "openai",
            "label": "GPT-5",
            "contextWindow": 200000,
            "kind": "chat",
        }),
        json!({
            "id": "gpt-4.1-mini",
            "provider": "openai",
            "label": "GPT-4.1 Mini",
            "contextWindow": 128000,
            "kind": "chat",
        }),
        json!({
            "id": "mock-local",
            "provider": "local",
            "label": "Mock Local",
            "contextWindow": 8192,
            "kind": "chat",
        }),
    ]
}
