use sqlx::{Executor, SqlitePool};

use crate::domain::error::DomainError;

pub async fn migrate(pool: &SqlitePool) -> Result<(), DomainError> {
    let migration = r#"
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;

    CREATE TABLE IF NOT EXISTS config_entries (
        key TEXT PRIMARY KEY NOT NULL,
        value_json TEXT NOT NULL,
        updated_at_ms INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS sessions (
        id TEXT PRIMARY KEY NOT NULL,
        title TEXT NOT NULL,
        tags_json TEXT NOT NULL,
        metadata_json TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL,
        updated_at_ms INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_sessions_updated_at ON sessions(updated_at_ms DESC);

    CREATE TABLE IF NOT EXISTS chat_messages (
        message_id TEXT PRIMARY KEY NOT NULL,
        session_key TEXT NOT NULL,
        role TEXT NOT NULL,
        text TEXT NOT NULL,
        status TEXT NOT NULL,
        metadata_json TEXT NOT NULL,
        ts_ms INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_chat_messages_session_ts ON chat_messages(session_key, ts_ms ASC);

    CREATE TABLE IF NOT EXISTS agent_runs (
        run_id TEXT PRIMARY KEY NOT NULL,
        agent_id TEXT NOT NULL,
        input TEXT NOT NULL,
        output TEXT NOT NULL,
        status TEXT NOT NULL,
        session_key TEXT,
        metadata_json TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL,
        updated_at_ms INTEGER NOT NULL,
        completed_at_ms INTEGER
    );
    CREATE INDEX IF NOT EXISTS idx_agent_runs_updated ON agent_runs(updated_at_ms DESC);

    CREATE TABLE IF NOT EXISTS cron_jobs (
        job_id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL,
        enabled INTEGER NOT NULL,
        schedule_json TEXT NOT NULL,
        payload_json TEXT NOT NULL,
        metadata_json TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL,
        updated_at_ms INTEGER NOT NULL,
        last_run_ms INTEGER,
        next_run_ms INTEGER
    );
    CREATE INDEX IF NOT EXISTS idx_cron_jobs_next_run ON cron_jobs(next_run_ms ASC);

    CREATE TABLE IF NOT EXISTS cron_runs (
        run_id TEXT PRIMARY KEY NOT NULL,
        job_id TEXT NOT NULL,
        status TEXT NOT NULL,
        output TEXT,
        error TEXT,
        manual INTEGER NOT NULL,
        started_at_ms INTEGER NOT NULL,
        finished_at_ms INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_cron_runs_job_started ON cron_runs(job_id, started_at_ms DESC);

    CREATE TABLE IF NOT EXISTS nodes (
        node_id TEXT PRIMARY KEY NOT NULL,
        display_name TEXT NOT NULL,
        platform TEXT NOT NULL,
        device_family TEXT,
        commands_json TEXT NOT NULL,
        paired INTEGER NOT NULL,
        status TEXT NOT NULL,
        last_seen_ms INTEGER NOT NULL,
        metadata_json TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_nodes_last_seen ON nodes(last_seen_ms DESC);

    CREATE TABLE IF NOT EXISTS node_pair_requests (
        request_id TEXT PRIMARY KEY NOT NULL,
        node_id TEXT NOT NULL,
        display_name TEXT NOT NULL,
        platform TEXT NOT NULL,
        device_family TEXT,
        commands_json TEXT NOT NULL,
        public_key TEXT,
        status TEXT NOT NULL,
        reason TEXT,
        created_at_ms INTEGER NOT NULL,
        resolved_at_ms INTEGER
    );
    CREATE INDEX IF NOT EXISTS idx_node_pair_requests_created ON node_pair_requests(created_at_ms DESC);

    CREATE TABLE IF NOT EXISTS node_invokes (
        invoke_id TEXT PRIMARY KEY NOT NULL,
        node_id TEXT NOT NULL,
        command TEXT NOT NULL,
        args_json TEXT NOT NULL,
        input_json TEXT,
        status TEXT NOT NULL,
        result_json TEXT,
        error TEXT,
        requested_at_ms INTEGER NOT NULL,
        updated_at_ms INTEGER NOT NULL,
        completed_at_ms INTEGER
    );
    CREATE INDEX IF NOT EXISTS idx_node_invokes_node_requested ON node_invokes(node_id, requested_at_ms DESC);

    CREATE TABLE IF NOT EXISTS node_events (
        event_id TEXT PRIMARY KEY NOT NULL,
        node_id TEXT NOT NULL,
        event TEXT NOT NULL,
        payload_json TEXT,
        ts_ms INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_node_events_node_ts ON node_events(node_id, ts_ms DESC);
    "#;

    pool.execute(migration)
        .await
        .map_err(|error| DomainError::Storage(format!("migration failed: {error}")))?;

    Ok(())
}
