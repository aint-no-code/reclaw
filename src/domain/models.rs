use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub metadata: Value,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub text: String,
    pub status: String,
    pub ts: u64,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatHistoryEntry {
    pub session_key: String,
    pub message: ChatMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunRecord {
    pub id: String,
    pub agent_id: String,
    pub input: String,
    pub output: String,
    pub status: String,
    pub session_key: Option<String>,
    pub metadata: Value,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub completed_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronSchedule {
    pub kind: String,
    pub at: Option<String>,
    pub every_ms: Option<u64>,
    pub anchor_ms: Option<u64>,
    pub expr: Option<String>,
    pub tz: Option<String>,
    pub stagger_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronPayload {
    pub kind: String,
    pub text: Option<String>,
    pub message: Option<String>,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJobRecord {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub schedule: CronSchedule,
    pub payload: CronPayload,
    pub metadata: Value,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub last_run_ms: Option<u64>,
    pub next_run_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronRunRecord {
    pub id: String,
    pub job_id: String,
    pub status: String,
    pub output: Option<String>,
    pub error: Option<String>,
    pub manual: bool,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct CronJobPatch {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub schedule: Option<CronSchedule>,
    pub payload: Option<CronPayload>,
    pub metadata: Option<Value>,
    pub next_run_ms: Option<Option<u64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeRecord {
    pub id: String,
    pub display_name: String,
    pub platform: String,
    pub device_family: Option<String>,
    pub commands: Vec<String>,
    pub paired: bool,
    pub status: String,
    pub last_seen_ms: u64,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodePairRequestRecord {
    pub request_id: String,
    pub node_id: String,
    pub display_name: String,
    pub platform: String,
    pub device_family: Option<String>,
    pub commands: Vec<String>,
    pub public_key: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub created_at_ms: u64,
    pub resolved_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInvokeRecord {
    pub request_id: String,
    pub node_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub input: Option<Value>,
    pub status: String,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub requested_at_ms: u64,
    pub updated_at_ms: u64,
    pub completed_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeEventRecord {
    pub id: String,
    pub node_id: String,
    pub event: String,
    pub payload: Option<Value>,
    pub ts: u64,
}

#[derive(Debug, Clone)]
pub struct NodePairRequestInput {
    pub node_id: String,
    pub display_name: String,
    pub platform: String,
    pub device_family: Option<String>,
    pub commands: Vec<String>,
    pub public_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NodeInvokeInput {
    pub node_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub input: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigEntry {
    pub key: String,
    pub value: Value,
    pub updated_at_ms: u64,
}
