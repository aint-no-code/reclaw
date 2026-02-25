use serde_json::{Value, json};

use crate::{
    domain::{error::DomainError, models::ConfigEntry},
    storage::SqliteStore,
};

impl SqliteStore {
    pub async fn load_config_doc(&self) -> Result<Value, DomainError> {
        let Some(entry) = self.get_config_entry("root").await? else {
            return Ok(json!({}));
        };

        if entry.value.is_object() {
            Ok(entry.value)
        } else {
            Ok(json!({}))
        }
    }

    pub async fn save_config_doc(&self, value: &Value) -> Result<(), DomainError> {
        if !value.is_object() {
            return Err(DomainError::InvalidRequest(
                "config payload must be an object".to_owned(),
            ));
        }

        let _ = self.set_config_entry("root", value).await?;

        Ok(())
    }

    pub async fn get_config_entry(&self, key: &str) -> Result<Option<ConfigEntry>, DomainError> {
        let row = sqlx::query_as::<_, (String, String, i64)>(
            "SELECT key, value_json, updated_at_ms FROM config_entries WHERE key = ? LIMIT 1",
        )
        .bind(key)
        .fetch_optional(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to read config entry: {error}")))?;

        row.map(map_config_entry_row).transpose()
    }

    pub async fn set_config_entry(
        &self,
        key: &str,
        value: &Value,
    ) -> Result<ConfigEntry, DomainError> {
        let json_text = serde_json::to_string(value).map_err(|error| {
            DomainError::Storage(format!("failed to serialize config value: {error}"))
        })?;
        let now = super::util::now_unix_ms();

        sqlx::query(
            "INSERT INTO config_entries(key, value_json, updated_at_ms) VALUES(?, ?, ?) \
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(key)
        .bind(json_text)
        .bind(i64::try_from(now).unwrap_or(i64::MAX))
        .execute(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to persist config entry: {error}")))?;

        Ok(ConfigEntry {
            key: key.to_owned(),
            value: value.clone(),
            updated_at_ms: now,
        })
    }

    pub async fn delete_config_entry(&self, key: &str) -> Result<bool, DomainError> {
        let result = sqlx::query("DELETE FROM config_entries WHERE key = ?")
            .bind(key)
            .execute(self.pool())
            .await
            .map_err(|error| {
                DomainError::Storage(format!("failed to delete config entry: {error}"))
            })?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn list_config_entries(
        &self,
        prefix: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ConfigEntry>, DomainError> {
        let mut query = String::from(
            "SELECT key, value_json, updated_at_ms FROM config_entries WHERE key LIKE ? ORDER BY updated_at_ms DESC",
        );
        if let Some(limit) = limit {
            query.push_str(" LIMIT ");
            query.push_str(&limit.to_string());
        }

        let pattern = format!("{prefix}%");
        let rows = sqlx::query_as::<_, (String, String, i64)>(&query)
            .bind(pattern)
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                DomainError::Storage(format!("failed to list config entries: {error}"))
            })?;

        rows.into_iter().map(map_config_entry_row).collect()
    }
}

fn map_config_entry_row(row: (String, String, i64)) -> Result<ConfigEntry, DomainError> {
    let (key, value_json, updated_at_ms) = row;
    let value = serde_json::from_str::<Value>(&value_json)
        .map_err(|error| DomainError::Storage(format!("invalid config entry JSON: {error}")))?;

    Ok(ConfigEntry {
        key,
        value,
        updated_at_ms: u64::try_from(updated_at_ms).unwrap_or(0),
    })
}
