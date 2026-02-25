use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("not paired: {0}")]
    NotPaired(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("unavailable: {0}")]
    Unavailable(String),
    #[error("storage error: {0}")]
    Storage(String),
}
