use crate::{
    domain::{error::DomainError, models::ChatMessage},
    storage::{SqliteStore, util},
};

impl SqliteStore {
    pub async fn append_chat_messages(
        &self,
        session_key: &str,
        messages: &[ChatMessage],
    ) -> Result<(), DomainError> {
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|error| DomainError::Storage(format!("failed to start tx: {error}")))?;

        for message in messages {
            let metadata_json =
                util::value_to_json_text(&message.metadata).map_err(DomainError::Storage)?;
            sqlx::query(
                "INSERT OR REPLACE INTO chat_messages(message_id, session_key, role, text, status, metadata_json, ts_ms) \
                 VALUES(?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&message.id)
            .bind(session_key)
            .bind(&message.role)
            .bind(&message.text)
            .bind(&message.status)
            .bind(metadata_json)
            .bind(i64::try_from(message.ts).unwrap_or(i64::MAX))
            .execute(&mut *tx)
            .await
            .map_err(|error| DomainError::Storage(format!("failed to insert chat message: {error}")))?;
        }

        tx.commit()
            .await
            .map_err(|error| DomainError::Storage(format!("failed to commit tx: {error}")))?;
        Ok(())
    }

    pub async fn list_chat_messages(
        &self,
        session_key: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ChatMessage>, DomainError> {
        let mut query = String::from(
            "SELECT message_id, role, text, status, metadata_json, ts_ms FROM chat_messages \
             WHERE session_key = ? ORDER BY ts_ms DESC",
        );

        if let Some(limit) = limit {
            query.push_str(" LIMIT ");
            query.push_str(&limit.to_string());
        }

        let rows = sqlx::query_as::<_, (String, String, String, String, String, i64)>(&query)
            .bind(session_key)
            .fetch_all(self.pool())
            .await
            .map_err(|error| {
                DomainError::Storage(format!("failed to list chat messages: {error}"))
            })?;

        let mut messages = rows
            .into_iter()
            .map(map_chat_row)
            .collect::<Result<Vec<_>, _>>()?;
        messages.reverse();
        Ok(messages)
    }

    pub async fn count_chat_messages(&self) -> Result<u64, DomainError> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM chat_messages")
            .fetch_one(self.pool())
            .await
            .map_err(|error| {
                DomainError::Storage(format!("failed to count chat messages: {error}"))
            })?;

        Ok(u64::try_from(count).unwrap_or(0))
    }
}

fn map_chat_row(
    row: (String, String, String, String, String, i64),
) -> Result<ChatMessage, DomainError> {
    let (id, role, text, status, metadata_json, ts_ms) = row;
    let metadata = util::json_text_to_value(&metadata_json).map_err(DomainError::Storage)?;
    Ok(ChatMessage {
        id,
        role,
        text,
        status,
        ts: u64::try_from(ts_ms).unwrap_or(0),
        metadata,
    })
}
