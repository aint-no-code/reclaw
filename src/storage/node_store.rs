use serde_json::Value;

use crate::{
    domain::{
        error::DomainError,
        models::{
            NodeEventRecord, NodeInvokeInput, NodeInvokeRecord, NodePairRequestInput,
            NodePairRequestRecord, NodeRecord,
        },
    },
    storage::{SqliteStore, util},
};

type NodeRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    i64,
    String,
    i64,
    String,
);

type NodePairRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    String,
    Option<String>,
    i64,
    Option<i64>,
);

type NodeInvokeRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    i64,
    i64,
    Option<i64>,
);

impl SqliteStore {
    pub async fn list_nodes(&self) -> Result<Vec<NodeRecord>, DomainError> {
        let rows = sqlx::query_as::<_, NodeRow>(
            "SELECT node_id, display_name, platform, device_family, commands_json, paired, status, last_seen_ms, metadata_json \
             FROM nodes ORDER BY last_seen_ms DESC",
        )
        .fetch_all(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to list nodes: {error}")))?;

        rows.into_iter().map(map_node_row).collect()
    }

    pub async fn get_node(&self, node_id: &str) -> Result<Option<NodeRecord>, DomainError> {
        let row = sqlx::query_as::<_, NodeRow>(
            "SELECT node_id, display_name, platform, device_family, commands_json, paired, status, last_seen_ms, metadata_json \
             FROM nodes WHERE node_id = ? LIMIT 1",
        )
        .bind(node_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to get node: {error}")))?;

        row.map(map_node_row).transpose()
    }

    pub async fn upsert_node(&self, node: &NodeRecord) -> Result<(), DomainError> {
        let commands_json = util::to_json_text(&node.commands).map_err(DomainError::Storage)?;
        let metadata_json =
            util::value_to_json_text(&node.metadata).map_err(DomainError::Storage)?;

        sqlx::query(
            "INSERT INTO nodes(node_id, display_name, platform, device_family, commands_json, paired, status, last_seen_ms, metadata_json) \
             VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(node_id) DO UPDATE SET \
               display_name = excluded.display_name, platform = excluded.platform, device_family = excluded.device_family, \
               commands_json = excluded.commands_json, paired = excluded.paired, status = excluded.status, \
               last_seen_ms = excluded.last_seen_ms, metadata_json = excluded.metadata_json",
        )
        .bind(&node.id)
        .bind(&node.display_name)
        .bind(&node.platform)
        .bind(&node.device_family)
        .bind(commands_json)
        .bind(if node.paired { 1_i64 } else { 0_i64 })
        .bind(&node.status)
        .bind(i64::try_from(node.last_seen_ms).unwrap_or(i64::MAX))
        .bind(metadata_json)
        .execute(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to upsert node: {error}")))?;
        Ok(())
    }

    pub async fn rename_node(
        &self,
        node_id: &str,
        display_name: &str,
    ) -> Result<NodeRecord, DomainError> {
        sqlx::query("UPDATE nodes SET display_name = ?, last_seen_ms = ? WHERE node_id = ?")
            .bind(display_name)
            .bind(i64::try_from(util::now_unix_ms()).unwrap_or(i64::MAX))
            .bind(node_id)
            .execute(self.pool())
            .await
            .map_err(|error| DomainError::Storage(format!("failed to rename node: {error}")))?;

        self.get_node(node_id)
            .await?
            .ok_or_else(|| DomainError::NotFound(format!("node not found: {node_id}")))
    }

    pub async fn add_node_pair_request(
        &self,
        input: NodePairRequestInput,
    ) -> Result<NodePairRequestRecord, DomainError> {
        let request = NodePairRequestRecord {
            request_id: format!("pair-{}", uuid::Uuid::new_v4()),
            node_id: input.node_id,
            display_name: input.display_name,
            platform: input.platform,
            device_family: input.device_family,
            commands: input.commands,
            public_key: input.public_key,
            status: "pending".to_owned(),
            reason: None,
            created_at_ms: util::now_unix_ms(),
            resolved_at_ms: None,
        };

        let commands_json = util::to_json_text(&request.commands).map_err(DomainError::Storage)?;
        sqlx::query(
            "INSERT INTO node_pair_requests(request_id, node_id, display_name, platform, device_family, commands_json, public_key, status, reason, created_at_ms, resolved_at_ms) \
             VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&request.request_id)
        .bind(&request.node_id)
        .bind(&request.display_name)
        .bind(&request.platform)
        .bind(&request.device_family)
        .bind(commands_json)
        .bind(&request.public_key)
        .bind(&request.status)
        .bind(&request.reason)
        .bind(i64::try_from(request.created_at_ms).unwrap_or(i64::MAX))
        .bind(Option::<i64>::None)
        .execute(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to insert pair request: {error}")))?;

        Ok(request)
    }

    pub async fn list_node_pair_requests(&self) -> Result<Vec<NodePairRequestRecord>, DomainError> {
        let rows = sqlx::query_as::<_, NodePairRow>(
            "SELECT request_id, node_id, display_name, platform, device_family, commands_json, public_key, status, reason, created_at_ms, resolved_at_ms \
             FROM node_pair_requests ORDER BY created_at_ms DESC",
        )
        .fetch_all(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to list pair requests: {error}")))?;

        rows.into_iter().map(map_pair_row).collect()
    }

    pub async fn resolve_node_pair_request(
        &self,
        request_id: &str,
        approved: bool,
        reason: Option<String>,
    ) -> Result<NodePairRequestRecord, DomainError> {
        let Some(mut request) = self.get_node_pair_request(request_id).await? else {
            return Err(DomainError::NotFound(format!(
                "pair request not found: {request_id}"
            )));
        };

        request.status = if approved { "approved" } else { "rejected" }.to_owned();
        request.reason = reason;
        request.resolved_at_ms = Some(util::now_unix_ms());

        sqlx::query("UPDATE node_pair_requests SET status = ?, reason = ?, resolved_at_ms = ? WHERE request_id = ?")
            .bind(&request.status)
            .bind(&request.reason)
            .bind(request.resolved_at_ms.map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
            .bind(request_id)
            .execute(self.pool())
            .await
            .map_err(|error| DomainError::Storage(format!("failed to resolve pair request: {error}")))?;

        let mut node = self
            .get_node(&request.node_id)
            .await?
            .unwrap_or(NodeRecord {
                id: request.node_id.clone(),
                display_name: request.display_name.clone(),
                platform: request.platform.clone(),
                device_family: request.device_family.clone(),
                commands: request.commands.clone(),
                paired: approved,
                status: "offline".to_owned(),
                last_seen_ms: util::now_unix_ms(),
                metadata: serde_json::json!({}),
            });

        node.display_name = request.display_name.clone();
        node.platform = request.platform.clone();
        node.device_family = request.device_family.clone();
        node.commands = request.commands.clone();
        node.paired = approved;
        node.last_seen_ms = util::now_unix_ms();
        self.upsert_node(&node).await?;

        Ok(request)
    }

    pub async fn create_node_invoke(
        &self,
        input: NodeInvokeInput,
    ) -> Result<NodeInvokeRecord, DomainError> {
        let Some(node) = self.get_node(&input.node_id).await? else {
            return Err(DomainError::NotFound(format!(
                "node not found: {}",
                input.node_id
            )));
        };
        if !node.paired {
            return Err(DomainError::NotPaired(format!(
                "node is not paired: {}",
                input.node_id
            )));
        }

        let now = util::now_unix_ms();
        let invoke = NodeInvokeRecord {
            request_id: format!("invoke-{}", uuid::Uuid::new_v4()),
            node_id: input.node_id,
            command: input.command,
            args: input.args,
            input: input.input,
            status: "completed".to_owned(),
            result: Some(serde_json::json!({
                "ok": true,
                "message": "invoke simulated by reclaw-server runtime"
            })),
            error: None,
            requested_at_ms: now,
            updated_at_ms: now,
            completed_at_ms: Some(now),
        };

        let args_json = util::to_json_text(&invoke.args).map_err(DomainError::Storage)?;
        let input_json = invoke
            .input
            .as_ref()
            .map(util::value_to_json_text)
            .transpose()
            .map_err(DomainError::Storage)?;
        let result_json = invoke
            .result
            .as_ref()
            .map(util::value_to_json_text)
            .transpose()
            .map_err(DomainError::Storage)?;

        sqlx::query(
            "INSERT INTO node_invokes(invoke_id, node_id, command, args_json, input_json, status, result_json, error, requested_at_ms, updated_at_ms, completed_at_ms) \
             VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&invoke.request_id)
        .bind(&invoke.node_id)
        .bind(&invoke.command)
        .bind(args_json)
        .bind(input_json)
        .bind(&invoke.status)
        .bind(result_json)
        .bind(&invoke.error)
        .bind(i64::try_from(invoke.requested_at_ms).unwrap_or(i64::MAX))
        .bind(i64::try_from(invoke.updated_at_ms).unwrap_or(i64::MAX))
        .bind(invoke.completed_at_ms.map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
        .execute(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to create node invoke: {error}")))?;

        Ok(invoke)
    }

    pub async fn update_node_invoke_result(
        &self,
        request_id: &str,
        status: String,
        payload: Option<Value>,
        error: Option<String>,
    ) -> Result<NodeInvokeRecord, DomainError> {
        let Some(mut invoke) = self.get_node_invoke(request_id).await? else {
            return Err(DomainError::NotFound(format!(
                "invoke request not found: {request_id}"
            )));
        };

        invoke.status = status;
        invoke.result = payload;
        invoke.error = error;
        invoke.updated_at_ms = util::now_unix_ms();
        if invoke.status == "completed" || invoke.status == "failed" {
            invoke.completed_at_ms = Some(invoke.updated_at_ms);
        }

        let result_json = invoke
            .result
            .as_ref()
            .map(util::value_to_json_text)
            .transpose()
            .map_err(DomainError::Storage)?;

        sqlx::query(
            "UPDATE node_invokes SET status = ?, result_json = ?, error = ?, updated_at_ms = ?, completed_at_ms = ? WHERE invoke_id = ?",
        )
        .bind(&invoke.status)
        .bind(result_json)
        .bind(&invoke.error)
        .bind(i64::try_from(invoke.updated_at_ms).unwrap_or(i64::MAX))
        .bind(invoke.completed_at_ms.map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
        .bind(request_id)
        .execute(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to update invoke result: {error}")))?;

        Ok(invoke)
    }

    pub async fn get_node_invoke(
        &self,
        request_id: &str,
    ) -> Result<Option<NodeInvokeRecord>, DomainError> {
        let row = sqlx::query_as::<_, NodeInvokeRow>(
            "SELECT invoke_id, node_id, command, args_json, input_json, status, result_json, error, requested_at_ms, updated_at_ms, completed_at_ms \
             FROM node_invokes WHERE invoke_id = ? LIMIT 1",
        )
        .bind(request_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to get invoke: {error}")))?;

        row.map(map_invoke_row).transpose()
    }

    pub async fn add_node_event(
        &self,
        node_id: String,
        event: String,
        payload: Option<Value>,
    ) -> Result<NodeEventRecord, DomainError> {
        let record = NodeEventRecord {
            id: format!("evt-{}", uuid::Uuid::new_v4()),
            node_id,
            event,
            payload,
            ts: util::now_unix_ms(),
        };

        let payload_json = record
            .payload
            .as_ref()
            .map(util::value_to_json_text)
            .transpose()
            .map_err(DomainError::Storage)?;

        sqlx::query(
            "INSERT INTO node_events(event_id, node_id, event, payload_json, ts_ms) VALUES(?, ?, ?, ?, ?)",
        )
        .bind(&record.id)
        .bind(&record.node_id)
        .bind(&record.event)
        .bind(payload_json)
        .bind(i64::try_from(record.ts).unwrap_or(i64::MAX))
        .execute(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to insert node event: {error}")))?;

        Ok(record)
    }

    pub async fn list_node_events(
        &self,
        node_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<NodeEventRecord>, DomainError> {
        let query = if node_id.is_some() {
            "SELECT event_id, node_id, event, payload_json, ts_ms FROM node_events WHERE node_id = ? ORDER BY ts_ms DESC"
        } else {
            "SELECT event_id, node_id, event, payload_json, ts_ms FROM node_events ORDER BY ts_ms DESC"
        };

        let mut rows = if let Some(node_id) = node_id {
            sqlx::query_as::<_, (String, String, String, Option<String>, i64)>(query)
                .bind(node_id)
                .fetch_all(self.pool())
                .await
                .map_err(|error| {
                    DomainError::Storage(format!("failed to list node events: {error}"))
                })?
        } else {
            sqlx::query_as::<_, (String, String, String, Option<String>, i64)>(query)
                .fetch_all(self.pool())
                .await
                .map_err(|error| {
                    DomainError::Storage(format!("failed to list node events: {error}"))
                })?
        }
        .into_iter()
        .map(map_event_row)
        .collect::<Result<Vec<_>, _>>()?;

        if let Some(limit) = limit {
            rows.truncate(limit);
        }
        Ok(rows)
    }

    pub async fn trim_node_events(&self, limit: usize) -> Result<(), DomainError> {
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT event_id FROM node_events ORDER BY ts_ms DESC LIMIT -1 OFFSET ?",
        )
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .fetch_all(self.pool())
        .await
        .map_err(|error| {
            DomainError::Storage(format!("failed to query old node events: {error}"))
        })?;

        if ids.is_empty() {
            return Ok(());
        }

        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|error| DomainError::Storage(format!("failed to start tx: {error}")))?;

        for event_id in ids {
            sqlx::query("DELETE FROM node_events WHERE event_id = ?")
                .bind(event_id)
                .execute(&mut *tx)
                .await
                .map_err(|error| {
                    DomainError::Storage(format!("failed to delete node event: {error}"))
                })?;
        }

        tx.commit()
            .await
            .map_err(|error| DomainError::Storage(format!("failed to commit tx: {error}")))?;
        Ok(())
    }

    async fn get_node_pair_request(
        &self,
        request_id: &str,
    ) -> Result<Option<NodePairRequestRecord>, DomainError> {
        let row = sqlx::query_as::<_, NodePairRow>(
            "SELECT request_id, node_id, display_name, platform, device_family, commands_json, public_key, status, reason, created_at_ms, resolved_at_ms \
             FROM node_pair_requests WHERE request_id = ? LIMIT 1",
        )
        .bind(request_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|error| DomainError::Storage(format!("failed to get pair request: {error}")))?;

        row.map(map_pair_row).transpose()
    }
}

fn map_node_row(row: NodeRow) -> Result<NodeRecord, DomainError> {
    let (
        id,
        display_name,
        platform,
        device_family,
        commands_json,
        paired,
        status,
        last_seen_ms,
        metadata_json,
    ) = row;

    let commands =
        util::from_json_text::<Vec<String>>(&commands_json).map_err(DomainError::Storage)?;
    let metadata = util::json_text_to_value(&metadata_json).map_err(DomainError::Storage)?;

    Ok(NodeRecord {
        id,
        display_name,
        platform,
        device_family,
        commands,
        paired: paired == 1,
        status,
        last_seen_ms: u64::try_from(last_seen_ms).unwrap_or(0),
        metadata,
    })
}

fn map_pair_row(row: NodePairRow) -> Result<NodePairRequestRecord, DomainError> {
    let (
        request_id,
        node_id,
        display_name,
        platform,
        device_family,
        commands_json,
        public_key,
        status,
        reason,
        created_at_ms,
        resolved_at_ms,
    ) = row;

    let commands =
        util::from_json_text::<Vec<String>>(&commands_json).map_err(DomainError::Storage)?;

    Ok(NodePairRequestRecord {
        request_id,
        node_id,
        display_name,
        platform,
        device_family,
        commands,
        public_key,
        status,
        reason,
        created_at_ms: u64::try_from(created_at_ms).unwrap_or(0),
        resolved_at_ms: resolved_at_ms.and_then(|value| u64::try_from(value).ok()),
    })
}

fn map_invoke_row(row: NodeInvokeRow) -> Result<NodeInvokeRecord, DomainError> {
    let (
        request_id,
        node_id,
        command,
        args_json,
        input_json,
        status,
        result_json,
        error,
        requested_at_ms,
        updated_at_ms,
        completed_at_ms,
    ) = row;

    let args = util::from_json_text::<Vec<String>>(&args_json).map_err(DomainError::Storage)?;
    let input = input_json
        .as_deref()
        .map(util::json_text_to_value)
        .transpose()
        .map_err(DomainError::Storage)?;
    let result = result_json
        .as_deref()
        .map(util::json_text_to_value)
        .transpose()
        .map_err(DomainError::Storage)?;

    Ok(NodeInvokeRecord {
        request_id,
        node_id,
        command,
        args,
        input,
        status,
        result,
        error,
        requested_at_ms: u64::try_from(requested_at_ms).unwrap_or(0),
        updated_at_ms: u64::try_from(updated_at_ms).unwrap_or(0),
        completed_at_ms: completed_at_ms.and_then(|value| u64::try_from(value).ok()),
    })
}

fn map_event_row(
    row: (String, String, String, Option<String>, i64),
) -> Result<NodeEventRecord, DomainError> {
    let (id, node_id, event, payload_json, ts_ms) = row;
    let payload = payload_json
        .as_deref()
        .map(util::json_text_to_value)
        .transpose()
        .map_err(DomainError::Storage)?;

    Ok(NodeEventRecord {
        id,
        node_id,
        event,
        payload,
        ts: u64::try_from(ts_ms).unwrap_or(0),
    })
}
