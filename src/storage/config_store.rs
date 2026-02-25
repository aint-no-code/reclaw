use serde_json::{Value, json};

use crate::{domain::error::DomainError, storage::SqliteStore};

impl SqliteStore {
    pub async fn load_config_doc(&self) -> Result<Value, DomainError> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT value_json FROM config_entries WHERE key = 'root' LIMIT 1",
        )
        .fetch_optional(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to read config: {error}")))?;

        let Some(json_text) = row else {
            return Ok(json!({}));
        };

        let value = serde_json::from_str::<Value>(&json_text)
            .map_err(|error| DomainError::Storage(format!("invalid config JSON: {error}")))?;

        if value.is_object() {
            Ok(value)
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

        let json_text = serde_json::to_string(value).map_err(|error| {
            DomainError::Storage(format!("failed to serialize config: {error}"))
        })?;
        let now = super::util::now_unix_ms();

        sqlx::query(
            "INSERT INTO config_entries(key, value_json, updated_at_ms) VALUES('root', ?, ?) \
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at_ms = excluded.updated_at_ms",
        )
        .bind(json_text)
        .bind(i64::try_from(now).unwrap_or(i64::MAX))
        .execute(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to persist config: {error}")))?;

        Ok(())
    }
}
