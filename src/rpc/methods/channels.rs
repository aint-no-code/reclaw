use std::collections::BTreeMap;

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
    #[serde(default)]
    account_id: Option<String>,
}

pub async fn handle_status(
    state: &SharedState,
    params: Option<&Value>,
) -> Result<Value, crate::protocol::ErrorShape> {
    let parsed: ChannelsStatusParams = parse_optional_params("channels.status", params)?;
    let include_disabled = parsed.include_disabled.unwrap_or(true);

    let persisted = state
        .get_config_entry_value(CHANNELS_STATUS_KEY)
        .await
        .map_err(map_domain_error)?
        .unwrap_or(Value::Array(Vec::new()));

    let mut channels = merge_channel_entries(
        configured_default_channels(state.config()),
        as_channel_array(persisted),
    );
    if !include_disabled {
        channels.retain(|item| {
            item.get("connected")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        });
    }
    let channel_views = build_channel_views(&channels);

    Ok(json!({
        "ts": now_unix_ms(),
        "channels": channels,
        "channelOrder": channel_views.channel_order,
        "channelLabels": channel_views.channel_labels,
        "channelMeta": channel_views.channel_meta,
        "channelsById": channel_views.channels_by_id,
        "channelAccounts": channel_views.channel_accounts,
        "channelDefaultAccountId": channel_views.channel_default_account_id,
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
    let requested_account_id = parsed
        .account_id
        .and_then(trim_non_empty)
        .unwrap_or_else(|| "default".to_owned());

    let mut channels = as_channel_array(
        state
            .get_config_entry_value(CHANNELS_STATUS_KEY)
            .await
            .map_err(map_domain_error)?
            .unwrap_or(Value::Array(Vec::new())),
    );
    channels = merge_channel_entries(configured_default_channels(state.config()), channels);

    let mut matched = false;
    for channel in &mut channels {
        let id_matches = channel
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == channel_id);
        if !id_matches {
            continue;
        }
        let channel_account_id =
            channel_account_id(channel).unwrap_or_else(|| "default".to_owned());
        if channel_account_id != requested_account_id {
            continue;
        }

        matched = true;
        if let Some(obj) = channel.as_object_mut() {
            obj.insert("connected".to_owned(), Value::Bool(false));
            obj.insert("loggedOutAtMs".to_owned(), Value::from(now_unix_ms()));
            obj.insert(
                "accountId".to_owned(),
                Value::from(requested_account_id.clone()),
            );
        }
    }

    if !matched {
        channels.push(json!({
            "id": channel_id,
            "accountId": requested_account_id,
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
        "accountId": requested_account_id,
        "loggedOut": true,
    }))
}

fn configured_default_channels(config: &crate::application::config::RuntimeConfig) -> Vec<Value> {
    let mut channels = BTreeMap::<String, Value>::new();
    channels.insert(
        "webchat".to_owned(),
        json!({
            "id": "webchat",
            "connected": true,
            "kind": "internal",
        }),
    );
    channels.insert(
        "node".to_owned(),
        json!({
            "id": "node",
            "connected": true,
            "kind": "gateway",
        }),
    );

    if config.telegram_webhook_secret.is_some() {
        channels.insert(
            "telegram".to_owned(),
            json!({
                "id": "telegram",
                "connected": true,
                "kind": "adapter",
            }),
        );
    }
    if config.discord_webhook_token.is_some() {
        channels.insert(
            "discord".to_owned(),
            json!({
                "id": "discord",
                "connected": true,
                "kind": "adapter",
            }),
        );
    }
    if config.slack_webhook_token.is_some() {
        channels.insert(
            "slack".to_owned(),
            json!({
                "id": "slack",
                "connected": true,
                "kind": "adapter",
            }),
        );
    }
    if config.signal_webhook_token.is_some() {
        channels.insert(
            "signal".to_owned(),
            json!({
                "id": "signal",
                "connected": true,
                "kind": "adapter",
            }),
        );
    }
    if config.whatsapp_webhook_token.is_some() {
        channels.insert(
            "whatsapp".to_owned(),
            json!({
                "id": "whatsapp",
                "connected": true,
                "kind": "adapter",
            }),
        );
    }
    for channel_id in config.channel_webhook_plugins.keys() {
        channels.entry(channel_id.clone()).or_insert_with(|| {
            json!({
                "id": channel_id,
                "connected": true,
                "kind": "plugin",
            })
        });
    }

    channels.into_values().collect()
}

fn merge_channel_entries(defaults: Vec<Value>, overrides: Vec<Value>) -> Vec<Value> {
    let mut merged = BTreeMap::<String, Value>::new();
    for entry in defaults {
        if let Some(key) = channel_entry_key(&entry) {
            merged.insert(key, entry);
        }
    }

    for entry in overrides {
        let Some(key) = channel_entry_key(&entry) else {
            continue;
        };

        match (merged.remove(&key), entry) {
            (Some(Value::Object(mut base)), Value::Object(overlay)) => {
                for (key, value) in overlay {
                    base.insert(key, value);
                }
                merged.insert(key, Value::Object(base));
            }
            (_, value) => {
                merged.insert(key, value);
            }
        }
    }

    merged.into_values().collect()
}

fn channel_entry_key(entry: &Value) -> Option<String> {
    let id = channel_id(entry)?;
    let account_id = channel_account_id(entry);
    match account_id.as_deref() {
        Some("default") | None => Some(id),
        Some(value) => Some(format!("{id}::{value}")),
    }
}

fn channel_account_id(entry: &Value) -> Option<String> {
    entry
        .get("accountId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn channel_connected(entry: &Value) -> bool {
    entry
        .get("connected")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn channel_kind(entry: &Value) -> String {
    entry
        .get("kind")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| "unknown".to_owned())
}

fn channel_label(channel_id: &str) -> String {
    channel_id.replace('-', " ")
}

struct ChannelViews {
    channel_order: Vec<Value>,
    channel_labels: Value,
    channel_meta: Value,
    channels_by_id: Value,
    channel_accounts: Value,
    channel_default_account_id: Value,
}

#[derive(Default)]
struct ChannelAggregate {
    connected: bool,
    kind: Option<String>,
    default_account_id: Option<String>,
}

fn build_channel_views(channels: &[Value]) -> ChannelViews {
    let mut order = Vec::new();
    let mut labels = Map::new();
    let mut meta = Map::new();
    let mut summaries = Map::new();
    let mut accounts = BTreeMap::<String, Vec<Value>>::new();
    let mut aggregates = BTreeMap::<String, ChannelAggregate>::new();
    let mut account_defaults = Map::new();

    for entry in channels {
        let Some(id) = channel_id(entry) else {
            continue;
        };
        let connected = channel_connected(entry);
        let account_id = channel_account_id(entry).unwrap_or_else(|| "default".to_owned());
        let kind = channel_kind(entry);
        let label = channel_label(&id);

        if !labels.contains_key(&id) {
            order.push(Value::from(id.clone()));
            labels.insert(id.clone(), Value::from(label.clone()));
            meta.insert(
                id.clone(),
                Value::Object(Map::from_iter([
                    ("kind".to_owned(), Value::from(kind.clone())),
                    ("label".to_owned(), Value::from(label)),
                ])),
            );
        }

        let account_entry = Value::Object(Map::from_iter([
            ("accountId".to_owned(), Value::from(account_id.clone())),
            ("connected".to_owned(), Value::from(connected)),
            ("kind".to_owned(), Value::from(kind.clone())),
            (
                "loggedOutAtMs".to_owned(),
                entry.get("loggedOutAtMs").cloned().unwrap_or(Value::Null),
            ),
        ]));
        accounts.entry(id.clone()).or_default().push(account_entry);

        let aggregate = aggregates.entry(id.clone()).or_default();
        aggregate.connected |= connected;
        if aggregate.kind.is_none() {
            aggregate.kind = Some(kind.clone());
        }
        if aggregate.default_account_id.is_none() || account_id == "default" {
            aggregate.default_account_id = Some(account_id.clone());
        }
    }

    for (id, aggregate) in aggregates {
        summaries.insert(
            id.clone(),
            Value::Object(Map::from_iter([
                ("connected".to_owned(), Value::from(aggregate.connected)),
                (
                    "kind".to_owned(),
                    Value::from(aggregate.kind.unwrap_or_else(|| "unknown".to_owned())),
                ),
            ])),
        );
        account_defaults.insert(
            id,
            Value::from(
                aggregate
                    .default_account_id
                    .unwrap_or_else(|| "default".to_owned()),
            ),
        );
    }

    let channel_accounts = Value::Object(
        accounts
            .into_iter()
            .map(|(id, entries)| (id, Value::Array(entries)))
            .collect(),
    );

    ChannelViews {
        channel_order: order,
        channel_labels: Value::Object(labels),
        channel_meta: Value::Object(meta),
        channels_by_id: Value::Object(summaries),
        channel_accounts,
        channel_default_account_id: Value::Object(account_defaults),
    }
}

fn channel_id(entry: &Value) -> Option<String> {
    entry
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
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

    use crate::application::config::{ChannelWebhookPluginConfig, RuntimeConfig};

    use super::{
        as_channel_array, build_channel_views, configured_default_channels, merge_channel_entries,
    };

    #[test]
    fn channel_array_supports_object_shape() {
        let value = json!({
            "slack": {"connected": true},
            "discord": {"connected": false}
        });

        let items = as_channel_array(value);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn configured_default_channels_include_plugin_entries() {
        let mut config = RuntimeConfig::for_test(
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            18_789,
            std::path::PathBuf::from(":memory:"),
        );
        config.slack_webhook_token = Some("slack-token".to_owned());
        config.channel_webhook_plugins.insert(
            "extchat".to_owned(),
            ChannelWebhookPluginConfig {
                url: "http://127.0.0.1:4900/webhook".to_owned(),
                token: None,
                timeout_ms: Some(3_000),
            },
        );

        let items = configured_default_channels(&config);
        assert!(items.iter().any(|entry| entry["id"] == "slack"
            && entry["kind"] == "adapter"
            && entry["connected"] == true));
        assert!(items.iter().any(|entry| entry["id"] == "extchat"
            && entry["kind"] == "plugin"
            && entry["connected"] == true));
    }

    #[test]
    fn merge_channel_entries_preserves_default_fields() {
        let defaults = vec![json!({
            "id": "slack",
            "connected": true,
            "kind": "adapter",
        })];
        let overrides = vec![json!({
            "id": "slack",
            "connected": false,
            "loggedOutAtMs": 123,
        })];

        let merged = merge_channel_entries(defaults, overrides);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["id"], "slack");
        assert_eq!(merged[0]["kind"], "adapter");
        assert_eq!(merged[0]["connected"], false);
        assert_eq!(merged[0]["loggedOutAtMs"], 123);
    }

    #[test]
    fn merge_channel_entries_supports_account_variants() {
        let defaults = vec![json!({
            "id": "slack",
            "connected": true,
            "kind": "adapter",
        })];
        let overrides = vec![
            json!({
                "id": "slack",
                "accountId": "ops",
                "connected": false,
                "loggedOutAtMs": 123,
            }),
            json!({
                "id": "slack",
                "accountId": "default",
                "connected": false,
                "loggedOutAtMs": 456,
            }),
        ];

        let merged = merge_channel_entries(defaults, overrides);
        assert_eq!(merged.len(), 2);
        assert!(merged.iter().any(|entry| {
            entry["id"] == "slack"
                && entry["accountId"] == "ops"
                && entry["connected"] == false
                && entry["loggedOutAtMs"] == 123
        }));
        assert!(merged.iter().any(|entry| {
            entry["id"] == "slack"
                && entry["accountId"] == "default"
                && entry["connected"] == false
                && entry["loggedOutAtMs"] == 456
                && entry["kind"] == "adapter"
        }));
    }

    #[test]
    fn build_channel_views_summarizes_accounts() {
        let channels = vec![
            json!({
                "id": "extchat",
                "connected": true,
                "kind": "plugin",
            }),
            json!({
                "id": "extchat",
                "accountId": "ops",
                "connected": false,
                "kind": "plugin",
                "loggedOutAtMs": 42,
            }),
        ];

        let views = build_channel_views(&channels);
        assert_eq!(views.channel_order, vec![json!("extchat")]);
        assert_eq!(views.channels_by_id["extchat"]["connected"], true);
        assert_eq!(views.channel_default_account_id["extchat"], "default");
        assert_eq!(views.channel_accounts["extchat"][0]["accountId"], "default");
        assert_eq!(views.channel_accounts["extchat"][1]["accountId"], "ops");
    }
}
