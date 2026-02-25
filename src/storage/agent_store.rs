use crate::{
    domain::{error::DomainError, models::AgentRunRecord},
    storage::{SqliteStore, util},
};

type AgentRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    i64,
    i64,
    Option<i64>,
);

impl SqliteStore {
    pub async fn upsert_agent_run(&self, run: &AgentRunRecord) -> Result<(), DomainError> {
        let metadata_json =
            util::value_to_json_text(&run.metadata).map_err(DomainError::Storage)?;
        sqlx::query(
            "INSERT INTO agent_runs(run_id, agent_id, input, output, status, session_key, metadata_json, created_at_ms, updated_at_ms, completed_at_ms) \
             VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(run_id) DO UPDATE SET \
               output = excluded.output, status = excluded.status, session_key = excluded.session_key, \
               metadata_json = excluded.metadata_json, updated_at_ms = excluded.updated_at_ms, \
               completed_at_ms = excluded.completed_at_ms",
        )
        .bind(&run.id)
        .bind(&run.agent_id)
        .bind(&run.input)
        .bind(&run.output)
        .bind(&run.status)
        .bind(&run.session_key)
        .bind(metadata_json)
        .bind(i64::try_from(run.created_at_ms).unwrap_or(i64::MAX))
        .bind(i64::try_from(run.updated_at_ms).unwrap_or(i64::MAX))
        .bind(run.completed_at_ms.map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
        .execute(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to upsert agent run: {error}")))?;
        Ok(())
    }

    pub async fn get_agent_run(&self, run_id: &str) -> Result<Option<AgentRunRecord>, DomainError> {
        let row = sqlx::query_as::<_, AgentRow>(
            "SELECT run_id, agent_id, input, output, status, session_key, metadata_json, created_at_ms, updated_at_ms, completed_at_ms \
             FROM agent_runs WHERE run_id = ? LIMIT 1",
        )
        .bind(run_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to get agent run: {error}")))?;

        row.map(map_agent_row).transpose()
    }

    pub async fn count_agent_runs(&self) -> Result<u64, DomainError> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM agent_runs")
            .fetch_one(self.pool())
            .await
            .map_err(|error| {
                DomainError::Storage(format!("failed to count agent runs: {error}"))
            })?;

        Ok(u64::try_from(count).unwrap_or(0))
    }
}

fn map_agent_row(row: AgentRow) -> Result<AgentRunRecord, DomainError> {
    let (
        id,
        agent_id,
        input,
        output,
        status,
        session_key,
        metadata_json,
        created_at_ms,
        updated_at_ms,
        completed_at_ms,
    ) = row;

    let metadata = util::json_text_to_value(&metadata_json).map_err(DomainError::Storage)?;

    Ok(AgentRunRecord {
        id,
        agent_id,
        input,
        output,
        status,
        session_key,
        metadata,
        created_at_ms: u64::try_from(created_at_ms).unwrap_or(0),
        updated_at_ms: u64::try_from(updated_at_ms).unwrap_or(0),
        completed_at_ms: completed_at_ms.and_then(|value| u64::try_from(value).ok()),
    })
}
