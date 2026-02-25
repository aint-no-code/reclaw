use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};

use clap::Parser;
use serde::Deserialize;

const DEFAULT_PORT: u16 = 18_789;
const DEFAULT_MAX_PAYLOAD_BYTES: usize = 25 * 1024 * 1024;
const DEFAULT_MAX_BUFFERED_BYTES: usize = 50 * 1024 * 1024;
const DEFAULT_HANDSHAKE_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_TICK_INTERVAL_MS: u64 = 30_000;
const DEFAULT_CRON_ENABLED: bool = true;
const DEFAULT_CRON_POLL_MS: u64 = 1_000;
const DEFAULT_CRON_RUNS_LIMIT: usize = 500;
const DEFAULT_AUTH_MAX_ATTEMPTS: u32 = 20;
const DEFAULT_AUTH_WINDOW_MS: u64 = 60_000;
const DEFAULT_LOG_FILTER: &str = "info";
const DEFAULT_JSON_LOGS: bool = false;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "reclaw-core",
    version,
    about = "Reclaw Core (Rust gateway transport + RPC core), forked from OpenClaw"
)]
pub struct Args {
    #[arg(long, env = "RECLAW_CONFIG")]
    pub config: Option<PathBuf>,

    #[arg(long, env = "RECLAW_HOST")]
    pub host: Option<IpAddr>,

    #[arg(long, env = "RECLAW_PORT")]
    pub port: Option<u16>,

    #[arg(long, env = "RECLAW_GATEWAY_TOKEN")]
    pub gateway_token: Option<String>,

    #[arg(long, env = "RECLAW_GATEWAY_PASSWORD")]
    pub gateway_password: Option<String>,

    #[arg(long, env = "RECLAW_MAX_PAYLOAD_BYTES")]
    pub max_payload_bytes: Option<usize>,

    #[arg(long, env = "RECLAW_MAX_BUFFERED_BYTES")]
    pub max_buffered_bytes: Option<usize>,

    #[arg(long, env = "RECLAW_HANDSHAKE_TIMEOUT_MS")]
    pub handshake_timeout_ms: Option<u64>,

    #[arg(long, env = "RECLAW_TICK_INTERVAL_MS")]
    pub tick_interval_ms: Option<u64>,

    #[arg(long, env = "RECLAW_CRON_ENABLED")]
    pub cron_enabled: Option<bool>,

    #[arg(long, env = "RECLAW_CRON_POLL_MS")]
    pub cron_poll_ms: Option<u64>,

    #[arg(long, env = "RECLAW_CRON_RUNS_LIMIT")]
    pub cron_runs_limit: Option<usize>,

    #[arg(long, env = "RECLAW_DB_PATH")]
    pub db_path: Option<PathBuf>,

    #[arg(long, env = "RECLAW_AUTH_MAX_ATTEMPTS")]
    pub auth_max_attempts: Option<u32>,

    #[arg(long, env = "RECLAW_AUTH_WINDOW_MS")]
    pub auth_window_ms: Option<u64>,

    #[arg(long, env = "RECLAW_RUNTIME_VERSION")]
    pub runtime_version: Option<String>,

    #[arg(long, env = "RUST_LOG")]
    pub log_filter: Option<String>,

    #[arg(long, env = "RECLAW_JSON_LOGS")]
    pub json_logs: Option<bool>,
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
        let static_paths = if let Some(explicit) = args.config.clone() {
            vec![explicit]
        } else {
            default_static_config_paths()
        };
        let static_config = load_static_config_from_paths(&static_paths)?;

        let host = args
            .host
            .or(static_config.host)
            .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let port = args.port.or(static_config.port).unwrap_or(DEFAULT_PORT);

        let max_payload_bytes = args
            .max_payload_bytes
            .or(static_config.max_payload_bytes)
            .unwrap_or(DEFAULT_MAX_PAYLOAD_BYTES);

        let max_buffered_bytes = args
            .max_buffered_bytes
            .or(static_config.max_buffered_bytes)
            .unwrap_or(DEFAULT_MAX_BUFFERED_BYTES);

        let handshake_timeout_ms = args
            .handshake_timeout_ms
            .or(static_config.handshake_timeout_ms)
            .unwrap_or(DEFAULT_HANDSHAKE_TIMEOUT_MS);

        let tick_interval_ms = args
            .tick_interval_ms
            .or(static_config.tick_interval_ms)
            .unwrap_or(DEFAULT_TICK_INTERVAL_MS);

        let cron_enabled = args
            .cron_enabled
            .or(static_config.cron_enabled)
            .unwrap_or(DEFAULT_CRON_ENABLED);

        let cron_poll_ms = args
            .cron_poll_ms
            .or(static_config.cron_poll_ms)
            .unwrap_or(DEFAULT_CRON_POLL_MS);

        let cron_runs_limit = args
            .cron_runs_limit
            .or(static_config.cron_runs_limit)
            .unwrap_or(DEFAULT_CRON_RUNS_LIMIT);

        let db_path = args
            .db_path
            .or(static_config.db_path)
            .unwrap_or_else(default_db_path);

        let auth_max_attempts = args
            .auth_max_attempts
            .or(static_config.auth_max_attempts)
            .unwrap_or(DEFAULT_AUTH_MAX_ATTEMPTS);

        let auth_window_ms = args
            .auth_window_ms
            .or(static_config.auth_window_ms)
            .unwrap_or(DEFAULT_AUTH_WINDOW_MS);

        let runtime_version =
            normalize_non_empty(args.runtime_version.or(static_config.runtime_version))
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());

        let log_filter = normalize_non_empty(args.log_filter.or(static_config.log_filter))
            .unwrap_or_else(|| DEFAULT_LOG_FILTER.to_owned());

        let json_logs = args
            .json_logs
            .or(static_config.json_logs)
            .unwrap_or(DEFAULT_JSON_LOGS);

        let auth_mode = resolve_auth_mode(
            args.gateway_token.or(static_config.gateway_token),
            args.gateway_password.or(static_config.gateway_password),
        )?;

        if port == 0 {
            return Err("port must be greater than 0".to_owned());
        }
        if max_payload_bytes == 0 {
            return Err("max_payload_bytes must be greater than 0".to_owned());
        }
        if max_buffered_bytes == 0 {
            return Err("max_buffered_bytes must be greater than 0".to_owned());
        }
        if auth_max_attempts == 0 {
            return Err("auth_max_attempts must be greater than 0".to_owned());
        }
        if cron_runs_limit == 0 {
            return Err("cron_runs_limit must be greater than 0".to_owned());
        }

        Ok(Self {
            host,
            port,
            auth_mode,
            max_payload_bytes,
            max_buffered_bytes,
            handshake_timeout: Duration::from_millis(handshake_timeout_ms),
            tick_interval_ms,
            cron_enabled,
            cron_poll_interval: Duration::from_millis(cron_poll_ms),
            cron_runs_limit,
            db_path,
            auth_max_attempts,
            auth_window: Duration::from_millis(auth_window_ms),
            runtime_version,
            log_filter,
            json_logs,
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct StaticConfigValues {
    host: Option<IpAddr>,
    port: Option<u16>,
    gateway_token: Option<String>,
    gateway_password: Option<String>,
    max_payload_bytes: Option<usize>,
    max_buffered_bytes: Option<usize>,
    handshake_timeout_ms: Option<u64>,
    tick_interval_ms: Option<u64>,
    cron_enabled: Option<bool>,
    cron_poll_ms: Option<u64>,
    cron_runs_limit: Option<usize>,
    db_path: Option<PathBuf>,
    auth_max_attempts: Option<u32>,
    auth_window_ms: Option<u64>,
    runtime_version: Option<String>,
    log_filter: Option<String>,
    json_logs: Option<bool>,
}

impl StaticConfigValues {
    fn merge_from(&mut self, other: Self) {
        override_option(&mut self.host, other.host);
        override_option(&mut self.port, other.port);
        override_option(&mut self.gateway_token, other.gateway_token);
        override_option(&mut self.gateway_password, other.gateway_password);
        override_option(&mut self.max_payload_bytes, other.max_payload_bytes);
        override_option(&mut self.max_buffered_bytes, other.max_buffered_bytes);
        override_option(&mut self.handshake_timeout_ms, other.handshake_timeout_ms);
        override_option(&mut self.tick_interval_ms, other.tick_interval_ms);
        override_option(&mut self.cron_enabled, other.cron_enabled);
        override_option(&mut self.cron_poll_ms, other.cron_poll_ms);
        override_option(&mut self.cron_runs_limit, other.cron_runs_limit);
        override_option(&mut self.db_path, other.db_path);
        override_option(&mut self.auth_max_attempts, other.auth_max_attempts);
        override_option(&mut self.auth_window_ms, other.auth_window_ms);
        override_option(&mut self.runtime_version, other.runtime_version);
        override_option(&mut self.log_filter, other.log_filter);
        override_option(&mut self.json_logs, other.json_logs);
    }
}

fn override_option<T>(slot: &mut Option<T>, value: Option<T>) {
    if let Some(value) = value {
        *slot = Some(value);
    }
}

fn default_static_config_paths() -> Vec<PathBuf> {
    let home = home_dir();
    default_static_config_paths_for(home.as_deref())
}

fn default_static_config_paths_for(home: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/etc/reclaw/config.toml"),
        PathBuf::from("/etc/reclaw/config.json"),
    ];

    if let Some(home) = home {
        paths.push(home.join(".reclaw/config.toml"));
        paths.push(home.join(".reclaw/config.json"));
    }

    paths
}

fn load_static_config_from_paths(paths: &[PathBuf]) -> Result<StaticConfigValues, String> {
    let mut merged = StaticConfigValues::default();

    for path in paths {
        if !path.exists() {
            continue;
        }

        let parsed = load_static_config_file(path)?;
        merged.merge_from(parsed);
    }

    Ok(merged)
}

fn load_static_config_file(path: &Path) -> Result<StaticConfigValues, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read config file {}: {error}", path.display()))?;

    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => serde_json::from_str::<StaticConfigValues>(&source)
            .map_err(|error| format!("invalid JSON config {}: {error}", path.display())),
        Some("toml") | None => toml::from_str::<StaticConfigValues>(&source)
            .map_err(|error| format!("invalid TOML config {}: {error}", path.display())),
        Some(extension) => Err(format!(
            "unsupported config extension for {}: .{extension}",
            path.display()
        )),
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

fn default_db_path() -> PathBuf {
    home_dir()
        .map(|home| home.join(".reclaw/reclaw.db"))
        .unwrap_or_else(|| PathBuf::from("./.reclaw-core/reclaw.db"))
}

fn normalize_non_empty(input: Option<String>) -> Option<String> {
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
    let token = normalize_non_empty(token);
    let password = normalize_non_empty(password);

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
    use std::{fs, net::IpAddr, net::Ipv4Addr};

    use super::{
        Args, AuthMode, RuntimeConfig, default_static_config_paths_for,
        load_static_config_from_paths, resolve_auth_mode,
    };

    fn empty_args() -> Args {
        Args {
            config: None,
            host: None,
            port: None,
            gateway_token: None,
            gateway_password: None,
            max_payload_bytes: None,
            max_buffered_bytes: None,
            handshake_timeout_ms: None,
            tick_interval_ms: None,
            cron_enabled: None,
            cron_poll_ms: None,
            cron_runs_limit: None,
            db_path: None,
            auth_max_attempts: None,
            auth_window_ms: None,
            runtime_version: None,
            log_filter: None,
            json_logs: None,
        }
    }

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

    #[test]
    fn default_paths_include_system_and_user_locations() {
        let home = std::path::Path::new("/home/tester");
        let paths = default_static_config_paths_for(Some(home));

        assert_eq!(
            paths[0],
            std::path::PathBuf::from("/etc/reclaw/config.toml")
        );
        assert_eq!(
            paths[1],
            std::path::PathBuf::from("/etc/reclaw/config.json")
        );
        assert_eq!(
            paths[2],
            std::path::PathBuf::from("/home/tester/.reclaw/config.toml")
        );
        assert_eq!(
            paths[3],
            std::path::PathBuf::from("/home/tester/.reclaw/config.json")
        );
    }

    #[test]
    fn static_config_merges_in_order() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let system_path = temp_dir.path().join("system.toml");
        let user_path = temp_dir.path().join("user.toml");

        fs::write(&system_path, "port = 19000\nmaxPayloadBytes = 100\n")
            .expect("system config should write");
        fs::write(&user_path, "port = 20000\n").expect("user config should write");

        let merged =
            load_static_config_from_paths(&[system_path, user_path]).expect("config should load");

        assert_eq!(merged.port, Some(20000));
        assert_eq!(merged.max_payload_bytes, Some(100));
    }

    #[test]
    fn runtime_config_uses_static_file_when_args_absent() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let config_path = temp_dir.path().join("config.toml");
        fs::write(
            &config_path,
            "host = \"0.0.0.0\"\nport = 19191\ncronEnabled = false\n",
        )
        .expect("config should write");

        let mut args = empty_args();
        args.config = Some(config_path);

        let runtime = RuntimeConfig::from_args(args).expect("runtime config should build");
        assert_eq!(runtime.host, IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)));
        assert_eq!(runtime.port, 19191);
        assert!(!runtime.cron_enabled);
    }

    #[test]
    fn runtime_config_cli_overrides_static_file() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let config_path = temp_dir.path().join("config.toml");
        fs::write(&config_path, "port = 19000\n").expect("config should write");

        let mut args = empty_args();
        args.config = Some(config_path);
        args.port = Some(20000);

        let runtime = RuntimeConfig::from_args(args).expect("runtime config should build");
        assert_eq!(runtime.port, 20000);
    }
}
