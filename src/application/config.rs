use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    time::Duration,
};

use clap::Parser;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "reclaw-core",
    version,
    about = "Reclaw Core (Rust gateway transport + RPC core), forked from OpenClaw"
)]
pub struct Args {
    #[arg(long, env = "RECLAW_HOST", default_value = "127.0.0.1")]
    pub host: IpAddr,

    #[arg(long, env = "RECLAW_PORT", default_value_t = 18789)]
    pub port: u16,

    #[arg(long, env = "RECLAW_GATEWAY_TOKEN")]
    pub gateway_token: Option<String>,

    #[arg(long, env = "RECLAW_GATEWAY_PASSWORD")]
    pub gateway_password: Option<String>,

    #[arg(long, env = "RECLAW_MAX_PAYLOAD_BYTES", default_value_t = 25 * 1024 * 1024)]
    pub max_payload_bytes: usize,

    #[arg(long, env = "RECLAW_MAX_BUFFERED_BYTES", default_value_t = 50 * 1024 * 1024)]
    pub max_buffered_bytes: usize,

    #[arg(long, env = "RECLAW_HANDSHAKE_TIMEOUT_MS", default_value_t = 10_000)]
    pub handshake_timeout_ms: u64,

    #[arg(long, env = "RECLAW_TICK_INTERVAL_MS", default_value_t = 30_000)]
    pub tick_interval_ms: u64,

    #[arg(long, env = "RECLAW_CRON_ENABLED", default_value_t = true)]
    pub cron_enabled: bool,

    #[arg(long, env = "RECLAW_CRON_POLL_MS", default_value_t = 1_000)]
    pub cron_poll_ms: u64,

    #[arg(long, env = "RECLAW_CRON_RUNS_LIMIT", default_value_t = 500)]
    pub cron_runs_limit: usize,

    #[arg(
        long,
        env = "RECLAW_DB_PATH",
        default_value = "./.reclaw-core/reclaw.db"
    )]
    pub db_path: PathBuf,

    #[arg(long, env = "RECLAW_AUTH_MAX_ATTEMPTS", default_value_t = 20)]
    pub auth_max_attempts: u32,

    #[arg(long, env = "RECLAW_AUTH_WINDOW_MS", default_value_t = 60_000)]
    pub auth_window_ms: u64,

    #[arg(long, env = "RECLAW_RUNTIME_VERSION", default_value = env!("CARGO_PKG_VERSION"))]
    pub runtime_version: String,

    #[arg(long, env = "RUST_LOG", default_value = "info")]
    pub log_filter: String,

    #[arg(long, env = "RECLAW_JSON_LOGS", default_value_t = false)]
    pub json_logs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMode {
    None,
    Token(String),
    Password(String),
}

impl AuthMode {
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Token(_) => "token",
            Self::Password(_) => "password",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub host: IpAddr,
    pub port: u16,
    pub auth_mode: AuthMode,
    pub max_payload_bytes: usize,
    pub max_buffered_bytes: usize,
    pub handshake_timeout: Duration,
    pub tick_interval_ms: u64,
    pub cron_enabled: bool,
    pub cron_poll_interval: Duration,
    pub cron_runs_limit: usize,
    pub db_path: PathBuf,
    pub auth_max_attempts: u32,
    pub auth_window: Duration,
    pub runtime_version: String,
    pub log_filter: String,
    pub json_logs: bool,
}

impl RuntimeConfig {
    pub fn from_args(args: Args) -> Result<Self, String> {
        let auth_mode = resolve_auth_mode(args.gateway_token, args.gateway_password)?;

        if args.port == 0 {
            return Err("port must be greater than 0".to_owned());
        }
        if args.max_payload_bytes == 0 {
            return Err("max_payload_bytes must be greater than 0".to_owned());
        }
        if args.max_buffered_bytes == 0 {
            return Err("max_buffered_bytes must be greater than 0".to_owned());
        }
        if args.auth_max_attempts == 0 {
            return Err("auth_max_attempts must be greater than 0".to_owned());
        }
        if args.cron_runs_limit == 0 {
            return Err("cron_runs_limit must be greater than 0".to_owned());
        }

        Ok(Self {
            host: args.host,
            port: args.port,
            auth_mode,
            max_payload_bytes: args.max_payload_bytes,
            max_buffered_bytes: args.max_buffered_bytes,
            handshake_timeout: Duration::from_millis(args.handshake_timeout_ms),
            tick_interval_ms: args.tick_interval_ms,
            cron_enabled: args.cron_enabled,
            cron_poll_interval: Duration::from_millis(args.cron_poll_ms),
            cron_runs_limit: args.cron_runs_limit,
            db_path: args.db_path,
            auth_max_attempts: args.auth_max_attempts,
            auth_window: Duration::from_millis(args.auth_window_ms),
            runtime_version: args.runtime_version,
            log_filter: args.log_filter,
            json_logs: args.json_logs,
        })
    }

    #[must_use]
    pub fn bind_addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }

    #[must_use]
    pub fn for_test(host: IpAddr, port: u16, db_path: PathBuf) -> Self {
        Self {
            host,
            port,
            auth_mode: AuthMode::None,
            max_payload_bytes: 512 * 1024,
            max_buffered_bytes: 1024 * 1024,
            handshake_timeout: Duration::from_millis(3_000),
            tick_interval_ms: 30_000,
            cron_enabled: true,
            cron_poll_interval: Duration::from_millis(200),
            cron_runs_limit: 100,
            db_path,
            auth_max_attempts: 3,
            auth_window: Duration::from_millis(5_000),
            runtime_version: "test".to_owned(),
            log_filter: "warn".to_owned(),
            json_logs: false,
        }
    }
}

fn normalize_secret(input: Option<String>) -> Option<String> {
    input.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn resolve_auth_mode(token: Option<String>, password: Option<String>) -> Result<AuthMode, String> {
    let token = normalize_secret(token);
    let password = normalize_secret(password);

    match (token, password) {
        (Some(_), Some(_)) => {
            Err("set either RECLAW_GATEWAY_TOKEN or RECLAW_GATEWAY_PASSWORD, not both".to_owned())
        }
        (Some(token), None) => Ok(AuthMode::Token(token)),
        (None, Some(password)) => Ok(AuthMode::Password(password)),
        (None, None) => Ok(AuthMode::None),
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthMode, resolve_auth_mode};

    #[test]
    fn auth_mode_rejects_dual_secrets() {
        let result = resolve_auth_mode(Some("a".to_owned()), Some("b".to_owned()));
        assert!(result.is_err());
    }

    #[test]
    fn auth_mode_trims_input() {
        let mode = resolve_auth_mode(Some(" token ".to_owned()), None).expect("auth mode expected");
        assert_eq!(mode, AuthMode::Token("token".to_owned()));
    }
}
