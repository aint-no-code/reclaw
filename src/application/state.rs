use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::{Map, Value, json};
use tokio::sync::RwLock;

use crate::{
    application::{config::RuntimeConfig, cron_schedule::compute_next_run_ms},
    domain::{
        error::DomainError,
        models::{
            AgentRunRecord, ChatMessage, ConfigEntry, CronJobPatch, CronJobRecord, CronRunRecord,
            NodeEventRecord, NodeInvokeInput, NodeInvokeRecord, NodePairRequestInput,
            NodePairRequestRecord, NodeRecord, SessionRecord,
        },
    },
    protocol::{PresenceEntry, Snapshot, StateVersion},
    security::rate_limit::AuthRateLimiter,
    storage::{SqliteStore, now_unix_ms},
};

#[derive(Clone)]
pub struct SharedState {
    inner: Arc<InnerState>,
}

struct InnerState {
    config: RuntimeConfig,
    store: SqliteStore,
    started_at: Instant,
    methods: Vec<String>,
    events: Vec<String>,
    clients: RwLock<HashMap<String, ConnectedClient>>,
    auth_rate_limiter: AuthRateLimiter,
    control_plane_rate_limiter: AuthRateLimiter,
    presence_version: AtomicU64,
    health_version: AtomicU64,
    cron_enabled: RwLock<bool>,
    cron_last_tick_ms: RwLock<Option<u64>>,
}

#[derive(Debug, Clone)]
pub struct ConnectedClient {
    pub conn_id: String,
    pub client_id: String,
    pub display_name: Option<String>,
    pub client_version: String,
    pub platform: String,
    pub device_family: Option<String>,
    pub model_identifier: Option<String>,
    pub mode: String,
    pub role: String,
    pub scopes: Vec<String>,
    pub instance_id: Option<String>,
    pub remote_ip: Option<String>,
    pub connected_at: Instant,
    pub connected_at_ms: u64,
}

impl SharedState {
    pub async fn new(
        config: RuntimeConfig,
        methods: Vec<String>,
        events: Vec<String>,
    ) -> Result<Self, DomainError> {
        let store = SqliteStore::connect(&config.db_path).await?;

        Ok(Self {
            inner: Arc::new(InnerState {
                auth_rate_limiter: AuthRateLimiter::new(
                    config.auth_max_attempts,
                    config.auth_window,
                ),
                control_plane_rate_limiter: AuthRateLimiter::new(3, Duration::from_secs(60)),
                started_at: Instant::now(),
                methods,
                events,
                clients: RwLock::new(HashMap::new()),
                store,
                cron_enabled: RwLock::new(config.cron_enabled),
                cron_last_tick_ms: RwLock::new(None),
                config,
                presence_version: AtomicU64::new(0),
                health_version: AtomicU64::new(0),
            }),
        })
    }

    #[must_use]
    pub fn config(&self) -> &RuntimeConfig {
        &self.inner.config
    }

    #[must_use]
    pub fn methods(&self) -> Vec<String> {
        self.inner.methods.clone()
    }

    #[must_use]
    pub fn events(&self) -> Vec<String> {
        self.inner.events.clone()
    }

    #[must_use]
    pub fn uptime_ms(&self) -> u64 {
        u64::try_from(self.inner.started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    #[must_use]
    pub fn auth_mode_label(&self) -> &'static str {
        self.inner.config.auth_mode.label()
    }

    #[must_use]
    pub fn auth_rate_limiter(&self) -> AuthRateLimiter {
        self.inner.auth_rate_limiter.clone()
    }

    #[must_use]
    pub fn control_plane_rate_limiter(&self) -> AuthRateLimiter {
        self.inner.control_plane_rate_limiter.clone()
    }

    pub async fn register_client(&self, client: ConnectedClient) -> Result<(), DomainError> {
        self.inner
            .clients
            .write()
            .await
            .insert(client.conn_id.clone(), client.clone());
        self.inner.presence_version.fetch_add(1, Ordering::Relaxed);

        if client.role == "node" {
            let node_id = runtime_node_id(&client);
            let node = NodeRecord {
                id: node_id.clone(),
                display_name: client
                    .display_name
                    .clone()
                    .unwrap_or_else(|| node_id.clone()),
                platform: client.platform.clone(),
                device_family: client.device_family.clone(),
                commands: Vec::new(),
                paired: true,
                status: "online".to_owned(),
                last_seen_ms: client.connected_at_ms,
                metadata: json!({
                    "remoteIp": client.remote_ip,
                    "modelIdentifier": client.model_identifier,
                    "version": client.client_version,
                }),
            };
            self.inner.store.upsert_node(&node).await?;
        }

        Ok(())
    }

    pub async fn unregister_client(&self, conn_id: &str) -> Result<(), DomainError> {
        let removed = self.inner.clients.write().await.remove(conn_id);
        if let Some(client) = removed {
            self.inner.presence_version.fetch_add(1, Ordering::Relaxed);
            if client.role == "node" {
                let node_id = runtime_node_id(&client);
                if let Some(mut node) = self.inner.store.get_node(&node_id).await? {
                    node.status = "offline".to_owned();
                    node.last_seen_ms = now_unix_ms();
                    self.inner.store.upsert_node(&node).await?;
                }
            }
        }
        Ok(())
    }

    pub async fn connection_count(&self) -> usize {
        self.inner.clients.read().await.len()
    }

    pub async fn health_payload(&self) -> Result<Value, DomainError> {
        let connections = self.connection_count().await;
        let sessions = self.inner.store.list_sessions().await?;
        let nodes = self.inner.store.list_nodes().await?;
        let jobs = self.inner.store.list_cron_jobs().await?;
        let chats = self
            .inner
            .store
            .list_chat_messages("agent:main:main", None)
            .await
            .unwrap_or_default();

        let health = json!({
            "ok": true,
            "ts": now_unix_ms(),
            "runtime": "rust",
            "version": self.config().runtime_version,
            "protocolVersion": crate::protocol::PROTOCOL_VERSION,
            "authMode": self.auth_mode_label(),
            "uptimeMs": self.uptime_ms(),
            "connectedClients": connections,
            "sessions": sessions.len(),
            "chatMessages": chats.len(),
            "cronJobs": jobs.len(),
            "nodes": nodes.len(),
        });

        self.inner.health_version.fetch_add(1, Ordering::Relaxed);
        Ok(health)
    }

    pub async fn snapshot(&self) -> Result<Snapshot, DomainError> {
        let health = self.health_payload().await?;

        Ok(Snapshot {
            presence: self.presence_entries().await,
            health,
            state_version: StateVersion {
                presence: self.inner.presence_version.load(Ordering::Relaxed),
                health: self.inner.health_version.load(Ordering::Relaxed),
            },
            uptime_ms: self.uptime_ms(),
            config_path: Some(self.config().db_path.display().to_string()),
            state_dir: self
                .config()
                .db_path
                .parent()
                .map(|path| path.display().to_string()),
            session_defaults: None,
            auth_mode: Some(self.auth_mode_label().to_owned()),
            update_available: None,
        })
    }

    pub async fn get_config_doc(&self) -> Result<Value, DomainError> {
        self.inner.store.load_config_doc().await
    }

    pub async fn set_config_doc(&self, next: Value) -> Result<(), DomainError> {
        self.inner.store.save_config_doc(&next).await
    }

    pub async fn get_config_entry_value(&self, key: &str) -> Result<Option<Value>, DomainError> {
        Ok(self
            .inner
            .store
            .get_config_entry(key)
            .await?
            .map(|entry| entry.value))
    }

    pub async fn set_config_entry_value(
        &self,
        key: &str,
        value: &Value,
    ) -> Result<ConfigEntry, DomainError> {
        self.inner.store.set_config_entry(key, value).await
    }

    pub async fn delete_config_entry_value(&self, key: &str) -> Result<bool, DomainError> {
        self.inner.store.delete_config_entry(key).await
    }

    pub async fn list_config_entries(
        &self,
        prefix: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ConfigEntry>, DomainError> {
        self.inner.store.list_config_entries(prefix, limit).await
    }

    pub async fn append_gateway_log(
        &self,
        level: &str,
        message: &str,
        method: Option<&str>,
        conn_id: Option<&str>,
    ) -> Result<(), DomainError> {
        let ts = now_unix_ms();
        let key = format!("logs/{ts}-{}", uuid::Uuid::new_v4());
        let entry = json!({
            "id": key,
            "level": level,
            "message": message,
            "method": method,
            "connId": conn_id,
            "ts": ts,
        });
        let _ = self.set_config_entry_value(&key, &entry).await?;
        Ok(())
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionRecord>, DomainError> {
        self.inner.store.list_sessions().await
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<SessionRecord>, DomainError> {
        self.inner.store.get_session(id).await
    }

    pub async fn upsert_session(&self, session: &SessionRecord) -> Result<(), DomainError> {
        self.inner.store.upsert_session(session).await
    }

    pub async fn remove_session(&self, id: &str) -> Result<bool, DomainError> {
        self.inner.store.remove_session(id).await
    }

    pub async fn clear_sessions(&self) -> Result<u64, DomainError> {
        self.inner.store.clear_sessions().await
    }

    pub async fn compact_sessions(&self, max_age_ms: u64) -> Result<u64, DomainError> {
        self.inner.store.compact_sessions(max_age_ms).await
    }

    pub async fn append_chat_messages(
        &self,
        session_key: &str,
        messages: &[ChatMessage],
    ) -> Result<(), DomainError> {
        self.inner
            .store
            .append_chat_messages(session_key, messages)
            .await
    }

    pub async fn list_chat_messages(
        &self,
        session_key: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ChatMessage>, DomainError> {
        self.inner
            .store
            .list_chat_messages(session_key, limit)
            .await
    }

    pub async fn count_chat_messages(&self) -> Result<u64, DomainError> {
        self.inner.store.count_chat_messages().await
    }

    pub async fn upsert_agent_run(&self, run: &AgentRunRecord) -> Result<(), DomainError> {
        self.inner.store.upsert_agent_run(run).await
    }

    pub async fn get_agent_run(&self, run_id: &str) -> Result<Option<AgentRunRecord>, DomainError> {
        self.inner.store.get_agent_run(run_id).await
    }

    pub async fn count_agent_runs(&self) -> Result<u64, DomainError> {
        self.inner.store.count_agent_runs().await
    }

    pub async fn list_cron_jobs(&self) -> Result<Vec<CronJobRecord>, DomainError> {
        self.inner.store.list_cron_jobs().await
    }

    pub async fn get_cron_job(&self, id: &str) -> Result<Option<CronJobRecord>, DomainError> {
        self.inner.store.get_cron_job(id).await
    }

    pub async fn add_cron_job(&self, job: &CronJobRecord) -> Result<(), DomainError> {
        self.inner.store.insert_cron_job(job).await
    }

    pub async fn update_cron_job(
        &self,
        id: &str,
        patch: CronJobPatch,
    ) -> Result<CronJobRecord, DomainError> {
        self.inner.store.update_cron_job(id, patch).await
    }

    pub async fn remove_cron_job(&self, id: &str) -> Result<bool, DomainError> {
        self.inner.store.remove_cron_job(id).await
    }

    pub async fn list_cron_runs(
        &self,
        job_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<CronRunRecord>, DomainError> {
        self.inner.store.list_cron_runs(job_id, limit).await
    }

    pub async fn cron_status(&self) -> Result<Value, DomainError> {
        let jobs = self.list_cron_jobs().await?;
        let runs = self.list_cron_runs(None, Some(50)).await?;
        let enabled = *self.inner.cron_enabled.read().await;
        let last_tick_ms = *self.inner.cron_last_tick_ms.read().await;

        Ok(json!({
            "enabled": enabled,
            "jobs": jobs,
            "runs": runs,
            "lastTickMs": last_tick_ms,
            "pollIntervalMs": self.config().cron_poll_interval.as_millis(),
            "storePath": self.config().db_path.display().to_string(),
        }))
    }

    pub async fn run_cron_job_now(&self, id: &str) -> Result<CronRunRecord, DomainError> {
        self.run_cron_job_internal(id, true).await
    }

    pub async fn tick_cron_jobs(&self) -> Result<usize, DomainError> {
        if !*self.inner.cron_enabled.read().await {
            return Ok(0);
        }

        let now = now_unix_ms();
        {
            let mut last_tick = self.inner.cron_last_tick_ms.write().await;
            *last_tick = Some(now);
        }

        let due_job_ids = self
            .list_cron_jobs()
            .await?
            .into_iter()
            .filter(|job| job.enabled && job.next_run_ms.is_some_and(|next| next <= now))
            .map(|job| job.id)
            .collect::<Vec<_>>();

        let mut executed = 0_usize;
        for id in due_job_ids {
            if self.run_cron_job_internal(&id, false).await.is_ok() {
                executed = executed.saturating_add(1);
            }
        }

        Ok(executed)
    }

    async fn run_cron_job_internal(
        &self,
        id: &str,
        manual: bool,
    ) -> Result<CronRunRecord, DomainError> {
        let Some(mut job) = self.get_cron_job(id).await? else {
            return Err(DomainError::NotFound(format!("cron job not found: {id}")));
        };

        let started = now_unix_ms();
        let result = execute_cron_payload(&job.payload, started);
        let finished = now_unix_ms();

        let (status, output, error) = match result {
            Ok(output) => ("ok".to_owned(), Some(output), None),
            Err(error) => ("error".to_owned(), None, Some(error)),
        };

        job.last_run_ms = Some(finished);
        job.updated_at_ms = finished;
        job.next_run_ms =
            compute_next_run_ms(&job.schedule, finished).map_err(DomainError::InvalidRequest)?;

        self.inner
            .store
            .update_cron_job(
                &job.id,
                CronJobPatch {
                    name: Some(job.name.clone()),
                    enabled: Some(job.enabled),
                    schedule: Some(job.schedule.clone()),
                    payload: Some(job.payload.clone()),
                    metadata: Some(job.metadata.clone()),
                    next_run_ms: Some(job.next_run_ms),
                },
            )
            .await?;

        let run = CronRunRecord {
            id: format!("run-{}", uuid::Uuid::new_v4()),
            job_id: job.id.clone(),
            status,
            output,
            error,
            manual,
            started_at_ms: started,
            finished_at_ms: finished,
        };

        self.inner.store.add_cron_run(&run).await?;
        self.inner
            .store
            .prune_cron_runs(self.config().cron_runs_limit)
            .await?;
        Ok(run)
    }

    pub async fn list_nodes(&self) -> Result<Vec<NodeRecord>, DomainError> {
        self.inner.store.list_nodes().await
    }

    pub async fn get_node(&self, id: &str) -> Result<Option<NodeRecord>, DomainError> {
        self.inner.store.get_node(id).await
    }

    pub async fn upsert_node(&self, node: &NodeRecord) -> Result<(), DomainError> {
        self.inner.store.upsert_node(node).await
    }

    pub async fn rename_node(
        &self,
        id: &str,
        display_name: &str,
    ) -> Result<NodeRecord, DomainError> {
        self.inner.store.rename_node(id, display_name).await
    }

    pub async fn add_node_pair_request(
        &self,
        input: NodePairRequestInput,
    ) -> Result<NodePairRequestRecord, DomainError> {
        self.inner.store.add_node_pair_request(input).await
    }

    pub async fn list_node_pair_requests(&self) -> Result<Vec<NodePairRequestRecord>, DomainError> {
        self.inner.store.list_node_pair_requests().await
    }

    pub async fn resolve_node_pair_request(
        &self,
        request_id: &str,
        approved: bool,
        reason: Option<String>,
    ) -> Result<NodePairRequestRecord, DomainError> {
        self.inner
            .store
            .resolve_node_pair_request(request_id, approved, reason)
            .await
    }

    pub async fn create_node_invoke(
        &self,
        input: NodeInvokeInput,
    ) -> Result<NodeInvokeRecord, DomainError> {
        self.inner.store.create_node_invoke(input).await
    }

    pub async fn update_node_invoke_result(
        &self,
        request_id: &str,
        status: String,
        payload: Option<Value>,
        error: Option<String>,
    ) -> Result<NodeInvokeRecord, DomainError> {
        self.inner
            .store
            .update_node_invoke_result(request_id, status, payload, error)
            .await
    }

    pub async fn get_node_invoke(
        &self,
        request_id: &str,
    ) -> Result<Option<NodeInvokeRecord>, DomainError> {
        self.inner.store.get_node_invoke(request_id).await
    }

    pub async fn add_node_event(
        &self,
        node_id: String,
        event: String,
        payload: Option<Value>,
    ) -> Result<NodeEventRecord, DomainError> {
        let record = self
            .inner
            .store
            .add_node_event(node_id, event, payload)
            .await?;
        self.inner.store.trim_node_events(500).await?;
        Ok(record)
    }

    pub async fn list_node_events(
        &self,
        node_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<NodeEventRecord>, DomainError> {
        self.inner.store.list_node_events(node_id, limit).await
    }

    async fn presence_entries(&self) -> Vec<PresenceEntry> {
        let now = Instant::now();
        self.inner
            .clients
            .read()
            .await
            .values()
            .map(|client| PresenceEntry {
                host: client
                    .display_name
                    .clone()
                    .or_else(|| Some(client.client_id.clone())),
                ip: client.remote_ip.clone(),
                version: Some(client.client_version.clone()),
                platform: Some(client.platform.clone()),
                device_family: client.device_family.clone(),
                model_identifier: client.model_identifier.clone(),
                mode: Some(client.mode.clone()),
                last_input_seconds: Some(now.duration_since(client.connected_at).as_secs()),
                reason: Some("connect".to_owned()),
                tags: None,
                text: None,
                ts: client.connected_at_ms,
                device_id: None,
                roles: Some(vec![client.role.clone()]),
                scopes: if client.scopes.is_empty() {
                    None
                } else {
                    Some(client.scopes.clone())
                },
                instance_id: client.instance_id.clone(),
            })
            .collect()
    }
}

fn execute_cron_payload(
    payload: &crate::domain::models::CronPayload,
    ts: u64,
) -> Result<String, String> {
    match payload.kind.as_str() {
        "systemEvent" => Ok(format!(
            "systemEvent:{} @{}",
            payload.text.clone().unwrap_or_default(),
            ts
        )),
        "agentTurn" => Ok(format!(
            "agentTurn:{} @{}",
            payload.message.clone().unwrap_or_default(),
            ts
        )),
        other => Err(format!("unsupported cron payload kind: {other}")),
    }
}

fn runtime_node_id(client: &ConnectedClient) -> String {
    client
        .instance_id
        .clone()
        .unwrap_or_else(|| client.client_id.clone())
}

pub fn sanitize_scopes(scopes: &[String]) -> Vec<String> {
    let mut unique = Map::new();
    for scope in scopes {
        let trimmed = scope.trim();
        if !trimmed.is_empty() {
            unique.insert(trimmed.to_owned(), Value::Bool(true));
        }
    }
    unique.keys().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::sanitize_scopes;

    #[test]
    fn sanitize_scopes_deduplicates_values() {
        let scopes = vec![
            "operator.admin".to_owned(),
            " operator.admin ".to_owned(),
            "".to_owned(),
            "operator.config.write".to_owned(),
        ];
        let sanitized = sanitize_scopes(&scopes);
        assert_eq!(sanitized.len(), 2);
        assert!(sanitized.contains(&"operator.admin".to_owned()));
        assert!(sanitized.contains(&"operator.config.write".to_owned()));
    }
}
