use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const ERROR_NOT_LINKED: &str = "NOT_LINKED";
pub const ERROR_NOT_PAIRED: &str = "NOT_PAIRED";
pub const ERROR_AGENT_TIMEOUT: &str = "AGENT_TIMEOUT";
pub const ERROR_INVALID_REQUEST: &str = "INVALID_REQUEST";
pub const ERROR_UNAVAILABLE: &str = "UNAVAILABLE";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ErrorShape {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl ErrorShape {
    #[must_use]
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            details: None,
            retryable: None,
            retry_after_ms: None,
        }
    }

    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    #[must_use]
    pub fn with_retry(mut self, retry_after_ms: u64) -> Self {
        self.retryable = Some(true);
        self.retry_after_ms = Some(retry_after_ms);
        self
    }
}
