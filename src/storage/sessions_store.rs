use crate::{
    domain::{error::DomainError, models::SessionRecord},
    storage::{SqliteStore, util},
};

impl SqliteStore {
    pub async fn list_sessions(&self) -> Result<Vec<SessionRecord>, DomainError> {
        let rows = sqlx::query_as::<_, (String, String, String, String, i64, i64)>(
            "SELECT id, title, tags_json, metadata_json, created_at_ms, updated_at_ms \
             FROM sessions ORDER BY updated_at_ms DESC",
        )
        .fetch_all(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to list sessions: {error}")))?;

        rows.into_iter().map(map_session_row).collect()
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<SessionRecord>, DomainError> {
        let row = sqlx::query_as::<_, (String, String, String, String, i64, i64)>(
            "SELECT id, title, tags_json, metadata_json, created_at_ms, updated_at_ms \
             FROM sessions WHERE id = ? LIMIT 1",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to get session: {error}")))?;

        row.map(map_session_row).transpose()
    }

    pub async fn upsert_session(&self, session: &SessionRecord) -> Result<(), DomainError> {
        let tags_json = util::to_json_text(&session.tags).map_err(DomainError::Storage)?;
        let metadata_json =
            util::value_to_json_text(&session.metadata).map_err(DomainError::Storage)?;

        sqlx::query(
            "INSERT INTO sessions(id, title, tags_json, metadata_json, created_at_ms, updated_at_ms) \
             VALUES(?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
               title = excluded.title, \
               tags_json = excluded.tags_json, \
               metadata_json = excluded.metadata_json, \
               updated_at_ms = excluded.updated_at_ms",
        )
        .bind(&session.id)
        .bind(&session.title)
        .bind(tags_json)
        .bind(metadata_json)
        .bind(i64::try_from(session.created_at_ms).unwrap_or(i64::MAX))
        .bind(i64::try_from(session.updated_at_ms).unwrap_or(i64::MAX))
        .execute(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to upsert session: {error}")))?;

        Ok(())
    }

    pub async fn remove_session(&self, id: &str) -> Result<bool, DomainError> {
        let result = sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(|error| DomainError::Storage(format!("failed to remove session: {error}")))?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn clear_sessions(&self) -> Result<u64, DomainError> {
        let result = sqlx::query("DELETE FROM sessions")
            .execute(self.pool())
            .await
            .map_err(|error| DomainError::Storage(format!("failed to clear sessions: {error}")))?;
        Ok(result.rows_affected())
    }

    pub async fn compact_sessions(&self, max_age_ms: u64) -> Result<u64, DomainError> {
        let now = util::now_unix_ms();
        let cutoff = now.saturating_sub(max_age_ms);
        let result = sqlx::query("DELETE FROM sessions WHERE updated_at_ms < ?")
            .bind(i64::try_from(cutoff).unwrap_or(i64::MAX))
            .execute(self.pool())
            .await
            .map_err(|error| {
                DomainError::Storage(format!("failed to compact sessions: {error}"))
            })?;
        Ok(result.rows_affected())
    }
}

fn map_session_row(
    row: (String, String, String, String, i64, i64),
) -> Result<SessionRecord, DomainError> {
    let (id, title, tags_json, metadata_json, created_at_ms, updated_at_ms) = row;
    let tags = util::from_json_text::<Vec<String>>(&tags_json).map_err(DomainError::Storage)?;
    let metadata = util::json_text_to_value(&metadata_json).map_err(DomainError::Storage)?;

    Ok(SessionRecord {
        id,
        title,
        tags,
        metadata,
        created_at_ms: u64::try_from(created_at_ms).unwrap_or(0),
        updated_at_ms: u64::try_from(updated_at_ms).unwrap_or(0),
    })
}
