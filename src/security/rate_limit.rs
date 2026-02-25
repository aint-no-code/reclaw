use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct RateLimitDecision {
    pub allowed: bool,
    pub retry_after_ms: u64,
}

#[derive(Debug, Clone)]
pub struct AuthRateLimiter {
    max_attempts: u32,
    window: Duration,
    state: Arc<RwLock<HashMap<String, Vec<u64>>>>,
}

impl AuthRateLimiter {
    #[must_use]
    pub fn new(max_attempts: u32, window: Duration) -> Self {
        Self {
            max_attempts,
            window,
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn check(&self, key: &str) -> RateLimitDecision {
        let now = now_unix_ms();
        let mut guard = self.state.write().await;
        let attempts = guard.entry(key.to_owned()).or_default();
        let cutoff = now.saturating_sub(self.window.as_millis() as u64);
        attempts.retain(|attempt| *attempt >= cutoff);

        if attempts.len() >= self.max_attempts as usize {
            let retry_after_ms = attempts
                .first()
                .map(|oldest| cutoff.saturating_sub(*oldest))
                .unwrap_or(0);
            return RateLimitDecision {
                allowed: false,
                retry_after_ms,
            };
        }

        RateLimitDecision {
            allowed: true,
            retry_after_ms: 0,
        }
    }

    pub async fn record_failure(&self, key: &str) -> RateLimitDecision {
        let now = now_unix_ms();
        let mut guard = self.state.write().await;
        let attempts = guard.entry(key.to_owned()).or_default();
        let cutoff = now.saturating_sub(self.window.as_millis() as u64);
        attempts.retain(|attempt| *attempt >= cutoff);
        attempts.push(now);

        if attempts.len() > self.max_attempts as usize {
            RateLimitDecision {
                allowed: false,
                retry_after_ms: self.window.as_millis() as u64,
            }
        } else {
            RateLimitDecision {
                allowed: true,
                retry_after_ms: 0,
            }
        }
    }

    pub async fn reset(&self, key: &str) {
        self.state.write().await.remove(key);
    }
}

fn now_unix_ms() -> u64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(value) => u64::try_from(value.as_millis()).unwrap_or(u64::MAX),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::AuthRateLimiter;

    #[tokio::test]
    async fn limiter_locks_after_threshold() {
        let limiter = AuthRateLimiter::new(2, Duration::from_secs(30));
        assert!(limiter.check("a").await.allowed);
        let _ = limiter.record_failure("a").await;
        let _ = limiter.record_failure("a").await;
        assert!(!limiter.record_failure("a").await.allowed);
    }
}
