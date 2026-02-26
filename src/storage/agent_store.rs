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

    pub async fn transition_agent_run_status(
        &self,
        run_id: &str,
        from_status: &str,
        to_status: &str,
        updated_at_ms: u64,
    ) -> Result<bool, DomainError> {
        let result = sqlx::query(
            "UPDATE agent_runs \
             SET status = ?, updated_at_ms = ? \
             WHERE run_id = ? AND status = ?",
        )
        .bind(to_status)
        .bind(i64::try_from(updated_at_ms).unwrap_or(i64::MAX))
        .bind(run_id)
        .bind(from_status)
        .execute(self.pool())
        .await
        .map_err(|error| {
            DomainError::Storage(format!("failed to transition agent run status: {error}"))
        })?;

        Ok(result.rows_affected() == 1)
    }

    pub async fn finalize_agent_run_if_status(
        &self,
        run: &AgentRunRecord,
        expected_status: &str,
    ) -> Result<bool, DomainError> {
        let metadata_json =
            util::value_to_json_text(&run.metadata).map_err(DomainError::Storage)?;
        let result = sqlx::query(
            "UPDATE agent_runs \
             SET output = ?, status = ?, session_key = ?, metadata_json = ?, updated_at_ms = ?, completed_at_ms = ? \
             WHERE run_id = ? AND status = ?",
        )
        .bind(&run.output)
        .bind(&run.status)
        .bind(&run.session_key)
        .bind(metadata_json)
        .bind(i64::try_from(run.updated_at_ms).unwrap_or(i64::MAX))
        .bind(run.completed_at_ms.map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
        .bind(&run.id)
        .bind(expected_status)
        .execute(self.pool())
        .await
        .map_err(|error| {
            DomainError::Storage(format!("failed to finalize agent run status: {error}"))
        })?;

        Ok(result.rows_affected() == 1)
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

    pub async fn list_agent_runs_by_session(
        &self,
        session_key: &str,
        limit: Option<usize>,
    ) -> Result<Vec<AgentRunRecord>, DomainError> {
        let limit = limit.unwrap_or(500).clamp(1, 5_000);
        let rows = sqlx::query_as::<_, AgentRow>(
            "SELECT run_id, agent_id, input, output, status, session_key, metadata_json, created_at_ms, updated_at_ms, completed_at_ms \
             FROM agent_runs WHERE session_key = ? ORDER BY updated_at_ms DESC LIMIT ?",
        )
        .bind(session_key)
        .bind(i64::try_from(limit).unwrap_or(5_000))
        .fetch_all(self.pool())
        .await
        .map_err(|error| {
            DomainError::Storage(format!("failed to list agent runs by session: {error}"))
        })?;

        rows.into_iter().map(map_agent_row).collect()
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

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::SqliteStore;
    use crate::{domain::models::AgentRunRecord, storage::now_unix_ms};
    use serde_json::json;

    async fn make_store() -> (TempDir, SqliteStore) {
        let temp = tempfile::tempdir().expect("temp dir should exist");
        let store = SqliteStore::connect(&temp.path().join("state.db"))
            .await
            .expect("sqlite store should connect");
        (temp, store)
    }

    fn run_with_status(run_id: &str, status: &str) -> AgentRunRecord {
        let now = now_unix_ms();
        AgentRunRecord {
            id: run_id.to_owned(),
            agent_id: "main".to_owned(),
            input: "hello".to_owned(),
            output: String::new(),
            status: status.to_owned(),
            session_key: Some("agent:main:test".to_owned()),
            metadata: json!({ "source": "test" }),
            created_at_ms: now,
            updated_at_ms: now,
            completed_at_ms: None,
        }
    }

    #[tokio::test]
    async fn transition_agent_run_status_claims_once() {
        let (_temp, store) = make_store().await;
        let queued = run_with_status("run-claim-1", "queued");
        store
            .upsert_agent_run(&queued)
            .await
            .expect("queued run should persist");

        let first = store
            .transition_agent_run_status(&queued.id, "queued", "running", now_unix_ms())
            .await
            .expect("first transition should succeed");
        let second = store
            .transition_agent_run_status(&queued.id, "queued", "running", now_unix_ms())
            .await
            .expect("second transition should be evaluated");

        assert!(first);
        assert!(!second);
    }

    #[tokio::test]
    async fn finalize_agent_run_respects_expected_status() {
        let (_temp, store) = make_store().await;
        let aborted = run_with_status("run-finalize-1", "aborted");
        store
            .upsert_agent_run(&aborted)
            .await
            .expect("aborted run should persist");

        let mut completed = aborted.clone();
        completed.status = "completed".to_owned();
        completed.output = "Echo: hello".to_owned();
        completed.updated_at_ms = now_unix_ms();
        completed.completed_at_ms = Some(completed.updated_at_ms);
        completed.metadata = json!({ "source": "test", "winner": "completed" });

        let finalized = store
            .finalize_agent_run_if_status(&completed, "running")
            .await
            .expect("finalize should return status");
        assert!(!finalized);

        let persisted = store
            .get_agent_run(&aborted.id)
            .await
            .expect("stored run should load")
            .expect("run should still exist");
        assert_eq!(persisted.status, "aborted");
        assert!(persisted.output.is_empty());
    }
}
