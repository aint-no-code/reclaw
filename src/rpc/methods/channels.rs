use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    application::state::SharedState,
    rpc::{
        dispatcher::map_domain_error,
        methods::{parse_optional_params, parse_required_params},
    },
    storage::now_unix_ms,
};

const CHANNELS_STATUS_KEY: &str = "runtime/channels/status";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChannelsStatusParams {
    #[serde(default)]
    include_disabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChannelsLogoutParams {
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

pub async fn handle_status(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ChannelsStatusParams = parse_optional_params("channels.status", params)?;
    let include_disabled = parsed.include_disabled.unwrap_or(true);

    let current = state
        .get_config_entry_value(CHANNELS_STATUS_KEY)
        .await
        .map_err(map_domain_error)?
        .unwrap_or_else(default_channels_status);

    let mut channels = as_channel_array(current);
    if !include_disabled {
        channels.retain(|item| {
            item.get("connected")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        });
    }

    Ok(json!({
        "ts": now_unix_ms(),
        "channels": channels,
    }))
}

pub async fn handle_logout(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ChannelsLogoutParams = parse_required_params("channels.logout", params)?;
    let channel_id = parsed
        .channel
        .or(parsed.id)
        .and_then(trim_non_empty)
        .ok_or_else(|| {
            crate::protocol::ErrorShape::new(
                crate::protocol::ERROR_INVALID_REQUEST,
                "invalid channels.logout params: channel is required",
            )
        })?;

    let mut channels = as_channel_array(
        state
            .get_config_entry_value(CHANNELS_STATUS_KEY)
            .await
            .map_err(map_domain_error)?
            .unwrap_or_else(default_channels_status),
    );

    let mut matched = false;
    for channel in &mut channels {
        let id_matches = channel
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == channel_id);
        if !id_matches {
            continue;
        }

        matched = true;
        if let Some(obj) = channel.as_object_mut() {
            obj.insert("connected".to_owned(), Value::Bool(false));
            obj.insert("loggedOutAtMs".to_owned(), Value::from(now_unix_ms()));
        }
    }

    if !matched {
        channels.push(json!({
            "id": channel_id,
            "connected": false,
            "loggedOutAtMs": now_unix_ms(),
        }));
    }

    let payload = Value::Array(channels);
    let _ = state
        .set_config_entry_value(CHANNELS_STATUS_KEY, &payload)
        .await
        .map_err(map_domain_error)?;

    Ok(json!({
        "ok": true,
        "channel": channel_id,
        "loggedOut": true,
    }))
}

fn default_channels_status() -> Value {
    Value::Array(vec![
        json!({
            "id": "webchat",
            "connected": true,
            "kind": "internal",
        }),
        json!({
            "id": "node",
            "connected": true,
            "kind": "gateway",
        }),
    ])
}

fn as_channel_array(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items,
        Value::Object(map) => map
            .into_iter()
            .map(|(id, data)| {
                if let Value::Object(mut obj) = data {
                    obj.entry("id".to_owned()).or_insert(Value::String(id));
                    Value::Object(obj)
                } else {
                    Value::Object(Map::from_iter([
                        ("id".to_owned(), Value::String(id)),
                        ("connected".to_owned(), Value::Bool(true)),
                    ]))
                }
            })
            .collect(),
        _ => Vec::new(),
    }
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
    use serde_json::json;

    use super::as_channel_array;

    #[test]
    fn channel_array_supports_object_shape() {
        let value = json!({
            "slack": {"connected": true},
            "discord": {"connected": false}
        });

        let items = as_channel_array(value);
        assert_eq!(items.len(), 2);
    }
}
