use crate::{
    domain::{
        error::DomainError,
        models::{CronJobPatch, CronJobRecord, CronPayload, CronRunRecord, CronSchedule},
    },
    storage::{SqliteStore, util},
};

type CronJobRow = (
    String,
    String,
    i64,
    String,
    String,
    String,
    i64,
    i64,
    Option<i64>,
    Option<i64>,
);

type CronRunRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    i64,
    i64,
);

impl SqliteStore {
    pub async fn list_cron_jobs(&self) -> Result<Vec<CronJobRecord>, DomainError> {
        let rows = sqlx::query_as::<_, CronJobRow>(
            "SELECT job_id, name, enabled, schedule_json, payload_json, metadata_json, created_at_ms, updated_at_ms, last_run_ms, next_run_ms \
             FROM cron_jobs ORDER BY name ASC",
        )
        .fetch_all(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to list cron jobs: {error}")))?;

        rows.into_iter().map(map_cron_job_row).collect()
    }

    pub async fn get_cron_job(&self, id: &str) -> Result<Option<CronJobRecord>, DomainError> {
        let row = sqlx::query_as::<_, CronJobRow>(
            "SELECT job_id, name, enabled, schedule_json, payload_json, metadata_json, created_at_ms, updated_at_ms, last_run_ms, next_run_ms \
             FROM cron_jobs WHERE job_id = ? LIMIT 1",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to get cron job: {error}")))?;

        row.map(map_cron_job_row).transpose()
    }

    pub async fn insert_cron_job(&self, job: &CronJobRecord) -> Result<(), DomainError> {
        let schedule_json = util::to_json_text(&job.schedule).map_err(DomainError::Storage)?;
        let payload_json = util::to_json_text(&job.payload).map_err(DomainError::Storage)?;
        let metadata_json =
            util::value_to_json_text(&job.metadata).map_err(DomainError::Storage)?;

        sqlx::query(
            "INSERT INTO cron_jobs(job_id, name, enabled, schedule_json, payload_json, metadata_json, created_at_ms, updated_at_ms, last_run_ms, next_run_ms) \
             VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&job.id)
        .bind(&job.name)
        .bind(if job.enabled { 1_i64 } else { 0_i64 })
        .bind(schedule_json)
        .bind(payload_json)
        .bind(metadata_json)
        .bind(i64::try_from(job.created_at_ms).unwrap_or(i64::MAX))
        .bind(i64::try_from(job.updated_at_ms).unwrap_or(i64::MAX))
        .bind(job.last_run_ms.map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
        .bind(job.next_run_ms.map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
        .execute(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to insert cron job: {error}")))?;
        Ok(())
    }

    pub async fn update_cron_job(
        &self,
        id: &str,
        patch: CronJobPatch,
    ) -> Result<CronJobRecord, DomainError> {
        let Some(mut existing) = self.get_cron_job(id).await? else {
            return Err(DomainError::NotFound(format!("cron job not found: {id}")));
        };

        if let Some(name) = patch.name {
            existing.name = name;
        }
        if let Some(enabled) = patch.enabled {
            existing.enabled = enabled;
        }
        if let Some(schedule) = patch.schedule {
            existing.schedule = schedule;
        }
        if let Some(payload) = patch.payload {
            existing.payload = payload;
        }
        if let Some(metadata) = patch.metadata {
            existing.metadata = metadata;
        }
        if let Some(next_run_ms) = patch.next_run_ms {
            existing.next_run_ms = next_run_ms;
        }
        existing.updated_at_ms = util::now_unix_ms();

        let schedule_json = util::to_json_text(&existing.schedule).map_err(DomainError::Storage)?;
        let payload_json = util::to_json_text(&existing.payload).map_err(DomainError::Storage)?;
        let metadata_json =
            util::value_to_json_text(&existing.metadata).map_err(DomainError::Storage)?;

        sqlx::query(
            "UPDATE cron_jobs SET name = ?, enabled = ?, schedule_json = ?, payload_json = ?, metadata_json = ?, \
             updated_at_ms = ?, last_run_ms = ?, next_run_ms = ? WHERE job_id = ?",
        )
        .bind(&existing.name)
        .bind(if existing.enabled { 1_i64 } else { 0_i64 })
        .bind(schedule_json)
        .bind(payload_json)
        .bind(metadata_json)
        .bind(i64::try_from(existing.updated_at_ms).unwrap_or(i64::MAX))
        .bind(existing.last_run_ms.map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
        .bind(existing.next_run_ms.map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
        .bind(&existing.id)
        .execute(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to update cron job: {error}")))?;

        Ok(existing)
    }

    pub async fn remove_cron_job(&self, id: &str) -> Result<bool, DomainError> {
        let result = sqlx::query("DELETE FROM cron_jobs WHERE job_id = ?")
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(|error| DomainError::Storage(format!("failed to remove cron job: {error}")))?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn add_cron_run(&self, run: &CronRunRecord) -> Result<(), DomainError> {
        sqlx::query(
            "INSERT INTO cron_runs(run_id, job_id, status, output, error, manual, started_at_ms, finished_at_ms) \
             VALUES(?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&run.id)
        .bind(&run.job_id)
        .bind(&run.status)
        .bind(&run.output)
        .bind(&run.error)
        .bind(if run.manual { 1_i64 } else { 0_i64 })
        .bind(i64::try_from(run.started_at_ms).unwrap_or(i64::MAX))
        .bind(i64::try_from(run.finished_at_ms).unwrap_or(i64::MAX))
        .execute(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to insert cron run: {error}")))?;
        Ok(())
    }

    pub async fn list_cron_runs(
        &self,
        job_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<CronRunRecord>, DomainError> {
        let query = if job_id.is_some() {
            "SELECT run_id, job_id, status, output, error, manual, started_at_ms, finished_at_ms \
             FROM cron_runs WHERE job_id = ? ORDER BY started_at_ms DESC"
        } else {
            "SELECT run_id, job_id, status, output, error, manual, started_at_ms, finished_at_ms \
             FROM cron_runs ORDER BY started_at_ms DESC"
        };

        let mut rows = if let Some(job_id) = job_id {
            sqlx::query_as::<_, CronRunRow>(query)
                .bind(job_id)
                .fetch_all(self.pool())
                .await
                .map_err(|error| {
                    DomainError::Storage(format!("failed to list cron runs: {error}"))
                })?
        } else {
            sqlx::query_as::<_, CronRunRow>(query)
                .fetch_all(self.pool())
                .await
                .map_err(|error| {
                    DomainError::Storage(format!("failed to list cron runs: {error}"))
                })?
        }
        .into_iter()
        .map(map_cron_run_row)
        .collect::<Result<Vec<_>, _>>()?;

        if let Some(limit) = limit {
            rows.truncate(limit);
        }
        Ok(rows)
    }

    pub async fn prune_cron_runs(&self, limit: usize) -> Result<(), DomainError> {
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT run_id FROM cron_runs ORDER BY started_at_ms DESC LIMIT -1 OFFSET ?",
        )
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .fetch_all(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to query prunable runs: {error}")))?;

        if ids.is_empty() {
            return Ok(());
        }

        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|error| DomainError::Storage(format!("failed to start tx: {error}")))?;

        for run_id in ids {
            sqlx::query("DELETE FROM cron_runs WHERE run_id = ?")
                .bind(run_id)
                .execute(&mut *tx)
                .await
                .map_err(|error| {
                    DomainError::Storage(format!("failed to prune cron run: {error}"))
                })?;
        }

        tx.commit()
            .await
            .map_err(|error| DomainError::Storage(format!("failed to commit tx: {error}")))?;
        Ok(())
    }

    pub async fn update_cron_job_runtime(
        &self,
        job_id: &str,
        last_run_ms: Option<u64>,
        next_run_ms: Option<u64>,
    ) -> Result<(), DomainError> {
        sqlx::query("UPDATE cron_jobs SET last_run_ms = ?, next_run_ms = ?, updated_at_ms = ? WHERE job_id = ?")
            .bind(last_run_ms.map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
            .bind(next_run_ms.map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
            .bind(i64::try_from(util::now_unix_ms()).unwrap_or(i64::MAX))
            .bind(job_id)
            .execute(self.pool())
            .await
            .map_err(|error| DomainError::Storage(format!("failed to update cron runtime: {error}")))?;
        Ok(())
    }
}

fn map_cron_job_row(row: CronJobRow) -> Result<CronJobRecord, DomainError> {
    let (
        id,
        name,
        enabled,
        schedule_json,
        payload_json,
        metadata_json,
        created_at_ms,
        updated_at_ms,
        last_run_ms,
        next_run_ms,
    ) = row;

    let schedule =
        util::from_json_text::<CronSchedule>(&schedule_json).map_err(DomainError::Storage)?;
    let payload =
        util::from_json_text::<CronPayload>(&payload_json).map_err(DomainError::Storage)?;
    let metadata = util::json_text_to_value(&metadata_json).map_err(DomainError::Storage)?;

    Ok(CronJobRecord {
        id,
        name,
        enabled: enabled == 1,
        schedule,
        payload,
        metadata,
        created_at_ms: u64::try_from(created_at_ms).unwrap_or(0),
        updated_at_ms: u64::try_from(updated_at_ms).unwrap_or(0),
        last_run_ms: last_run_ms.and_then(|value| u64::try_from(value).ok()),
        next_run_ms: next_run_ms.and_then(|value| u64::try_from(value).ok()),
    })
}

fn map_cron_run_row(row: CronRunRow) -> Result<CronRunRecord, DomainError> {
    let (id, job_id, status, output, error, manual, started_at_ms, finished_at_ms) = row;
    Ok(CronRunRecord {
        id,
        job_id,
        status,
        output,
        error,
        manual: manual == 1,
        started_at_ms: u64::try_from(started_at_ms).unwrap_or(0),
        finished_at_ms: u64::try_from(finished_at_ms).unwrap_or(0),
    })
}
