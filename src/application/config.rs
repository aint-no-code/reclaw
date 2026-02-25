use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};

use clap::{Parser, Subcommand, ValueEnum};
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
const DEFAULT_HOOKS_PATH: &str = "/hooks";
const DEFAULT_HOOKS_MAX_BODY_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "reclaw-core",
    version,
    about = "Reclaw Core (Rust gateway transport + RPC core), forked from OpenClaw"
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,

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

    #[arg(long, env = "RECLAW_CHANNELS_INBOUND_TOKEN")]
    pub channels_inbound_token: Option<String>,

    #[arg(long, env = "RECLAW_TELEGRAM_WEBHOOK_SECRET")]
    pub telegram_webhook_secret: Option<String>,

    #[arg(long, env = "RECLAW_TELEGRAM_BOT_TOKEN")]
    pub telegram_bot_token: Option<String>,

    #[arg(long, env = "RECLAW_TELEGRAM_API_BASE_URL")]
    pub telegram_api_base_url: Option<String>,

    #[arg(long, env = "RECLAW_DISCORD_WEBHOOK_TOKEN")]
    pub discord_webhook_token: Option<String>,

    #[arg(long, env = "RECLAW_DISCORD_OUTBOUND_URL")]
    pub discord_outbound_url: Option<String>,

    #[arg(long, env = "RECLAW_DISCORD_OUTBOUND_TOKEN")]
    pub discord_outbound_token: Option<String>,

    #[arg(long, env = "RECLAW_SLACK_WEBHOOK_TOKEN")]
    pub slack_webhook_token: Option<String>,

    #[arg(long, env = "RECLAW_SLACK_OUTBOUND_URL")]
    pub slack_outbound_url: Option<String>,

    #[arg(long, env = "RECLAW_SLACK_OUTBOUND_TOKEN")]
    pub slack_outbound_token: Option<String>,

    #[arg(long, env = "RECLAW_SIGNAL_WEBHOOK_TOKEN")]
    pub signal_webhook_token: Option<String>,

    #[arg(long, env = "RECLAW_SIGNAL_OUTBOUND_URL")]
    pub signal_outbound_url: Option<String>,

    #[arg(long, env = "RECLAW_SIGNAL_OUTBOUND_TOKEN")]
    pub signal_outbound_token: Option<String>,

    #[arg(long, env = "RECLAW_WHATSAPP_WEBHOOK_TOKEN")]
    pub whatsapp_webhook_token: Option<String>,

    #[arg(long, env = "RECLAW_WHATSAPP_OUTBOUND_URL")]
    pub whatsapp_outbound_url: Option<String>,

    #[arg(long, env = "RECLAW_WHATSAPP_OUTBOUND_TOKEN")]
    pub whatsapp_outbound_token: Option<String>,

    #[arg(long, env = "RECLAW_OPENAI_CHAT_COMPLETIONS_ENABLED")]
    pub openai_chat_completions_enabled: Option<bool>,

    #[arg(long, env = "RECLAW_OPENRESPONSES_ENABLED")]
    pub openresponses_enabled: Option<bool>,

    #[arg(long, env = "RECLAW_HOOKS_ENABLED")]
    pub hooks_enabled: Option<bool>,

    #[arg(long, env = "RECLAW_HOOKS_TOKEN")]
    pub hooks_token: Option<String>,

    #[arg(long, env = "RECLAW_HOOKS_PATH")]
    pub hooks_path: Option<String>,

    #[arg(long, env = "RECLAW_HOOKS_MAX_BODY_BYTES")]
    pub hooks_max_body_bytes: Option<usize>,

    #[arg(long, env = "RECLAW_HOOKS_ALLOW_REQUEST_SESSION_KEY")]
    pub hooks_allow_request_session_key: Option<bool>,

    #[arg(long, env = "RECLAW_HOOKS_DEFAULT_SESSION_KEY")]
    pub hooks_default_session_key: Option<String>,

    #[arg(long, env = "RECLAW_HOOKS_DEFAULT_AGENT_ID")]
    pub hooks_default_agent_id: Option<String>,

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

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Initialize static config directories and base config files.
    InitConfig(InitConfigArgs),
}

#[derive(Debug, Clone, clap::Args)]
pub struct InitConfigArgs {
    /// Where to initialize config files.
    #[arg(long, value_enum, default_value_t = InitScope::User)]
    pub scope: InitScope,

    /// Run without prompts; create missing config files automatically.
    #[arg(long, default_value_t = false)]
    pub non_interactive: bool,

    /// Overwrite existing config files.
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InitScope {
    User,
    System,
    Both,
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
    pub channels_inbound_token: Option<String>,
    pub telegram_webhook_secret: Option<String>,
    pub telegram_bot_token: Option<String>,
    pub telegram_api_base_url: String,
    pub discord_webhook_token: Option<String>,
    pub discord_outbound_url: Option<String>,
    pub discord_outbound_token: Option<String>,
    pub slack_webhook_token: Option<String>,
    pub slack_outbound_url: Option<String>,
    pub slack_outbound_token: Option<String>,
    pub signal_webhook_token: Option<String>,
    pub signal_outbound_url: Option<String>,
    pub signal_outbound_token: Option<String>,
    pub whatsapp_webhook_token: Option<String>,
    pub whatsapp_outbound_url: Option<String>,
    pub whatsapp_outbound_token: Option<String>,
    pub hooks_enabled: bool,
    pub hooks_token: Option<String>,
    pub hooks_path: String,
    pub hooks_max_body_bytes: usize,
    pub hooks_allow_request_session_key: bool,
    pub hooks_default_session_key: Option<String>,
    pub hooks_default_agent_id: String,
    pub openai_chat_completions_enabled: bool,
    pub openresponses_enabled: bool,
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
        if args.command.is_some() {
            return Err("command mode does not produce runtime configuration".to_owned());
        }

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

        let channels_inbound_token = normalize_non_empty(
            args.channels_inbound_token
                .or(static_config.channels_inbound_token),
        );
        let telegram_webhook_secret = normalize_non_empty(
            args.telegram_webhook_secret
                .or(static_config.telegram_webhook_secret),
        );
        let telegram_bot_token =
            normalize_non_empty(args.telegram_bot_token.or(static_config.telegram_bot_token));
        let telegram_api_base_url = normalize_non_empty(
            args.telegram_api_base_url
                .or(static_config.telegram_api_base_url),
        )
        .unwrap_or_else(|| "https://api.telegram.org".to_owned());
        let discord_webhook_token = normalize_non_empty(
            args.discord_webhook_token
                .or(static_config.discord_webhook_token),
        );
        let discord_outbound_url = normalize_non_empty(
            args.discord_outbound_url
                .or(static_config.discord_outbound_url),
        );
        let discord_outbound_token = normalize_non_empty(
            args.discord_outbound_token
                .or(static_config.discord_outbound_token),
        );
        let slack_webhook_token = normalize_non_empty(
            args.slack_webhook_token
                .or(static_config.slack_webhook_token),
        );
        let slack_outbound_url =
            normalize_non_empty(args.slack_outbound_url.or(static_config.slack_outbound_url));
        let slack_outbound_token = normalize_non_empty(
            args.slack_outbound_token
                .or(static_config.slack_outbound_token),
        );
        let signal_webhook_token = normalize_non_empty(
            args.signal_webhook_token
                .or(static_config.signal_webhook_token),
        );
        let signal_outbound_url = normalize_non_empty(
            args.signal_outbound_url
                .or(static_config.signal_outbound_url),
        );
        let signal_outbound_token = normalize_non_empty(
            args.signal_outbound_token
                .or(static_config.signal_outbound_token),
        );
        let whatsapp_webhook_token = normalize_non_empty(
            args.whatsapp_webhook_token
                .or(static_config.whatsapp_webhook_token),
        );
        let whatsapp_outbound_url = normalize_non_empty(
            args.whatsapp_outbound_url
                .or(static_config.whatsapp_outbound_url),
        );
        let whatsapp_outbound_token = normalize_non_empty(
            args.whatsapp_outbound_token
                .or(static_config.whatsapp_outbound_token),
        );
        let hooks_enabled = args
            .hooks_enabled
            .or(static_config.hooks_enabled)
            .unwrap_or(false);
        let hooks_token = normalize_non_empty(args.hooks_token.or(static_config.hooks_token));
        let hooks_path = normalize_hooks_path(
            args.hooks_path
                .or(static_config.hooks_path)
                .unwrap_or_else(|| DEFAULT_HOOKS_PATH.to_owned()),
        )?;
        let hooks_max_body_bytes = args
            .hooks_max_body_bytes
            .or(static_config.hooks_max_body_bytes)
            .unwrap_or(DEFAULT_HOOKS_MAX_BODY_BYTES);
        let hooks_allow_request_session_key = args
            .hooks_allow_request_session_key
            .or(static_config.hooks_allow_request_session_key)
            .unwrap_or(false);
        let hooks_default_session_key = normalize_non_empty(
            args.hooks_default_session_key
                .or(static_config.hooks_default_session_key),
        );
        let hooks_default_agent_id = normalize_non_empty(
            args.hooks_default_agent_id
                .or(static_config.hooks_default_agent_id),
        )
        .unwrap_or_else(|| "main".to_owned());
        if hooks_enabled && hooks_token.is_none() {
            return Err("hooks.enabled requires hooks.token".to_owned());
        }
        let openai_chat_completions_enabled = args
            .openai_chat_completions_enabled
            .or(static_config.openai_chat_completions_enabled)
            .unwrap_or(false);
        let openresponses_enabled = args
            .openresponses_enabled
            .or(static_config.openresponses_enabled)
            .unwrap_or(false);

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
        if hooks_max_body_bytes == 0 {
            return Err("hooks_max_body_bytes must be greater than 0".to_owned());
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
            channels_inbound_token,
            telegram_webhook_secret,
            telegram_bot_token,
            telegram_api_base_url,
            discord_webhook_token,
            discord_outbound_url,
            discord_outbound_token,
            slack_webhook_token,
            slack_outbound_url,
            slack_outbound_token,
            signal_webhook_token,
            signal_outbound_url,
            signal_outbound_token,
            whatsapp_webhook_token,
            whatsapp_outbound_url,
            whatsapp_outbound_token,
            hooks_enabled,
            hooks_token,
            hooks_path,
            hooks_max_body_bytes,
            hooks_allow_request_session_key,
            hooks_default_session_key,
            hooks_default_agent_id,
            openai_chat_completions_enabled,
            openresponses_enabled,
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
            channels_inbound_token: None,
            telegram_webhook_secret: None,
            telegram_bot_token: None,
            telegram_api_base_url: "https://api.telegram.org".to_owned(),
            discord_webhook_token: None,
            discord_outbound_url: None,
            discord_outbound_token: None,
            slack_webhook_token: None,
            slack_outbound_url: None,
            slack_outbound_token: None,
            signal_webhook_token: None,
            signal_outbound_url: None,
            signal_outbound_token: None,
            whatsapp_webhook_token: None,
            whatsapp_outbound_url: None,
            whatsapp_outbound_token: None,
            hooks_enabled: false,
            hooks_token: None,
            hooks_path: DEFAULT_HOOKS_PATH.to_owned(),
            hooks_max_body_bytes: DEFAULT_HOOKS_MAX_BODY_BYTES,
            hooks_allow_request_session_key: false,
            hooks_default_session_key: None,
            hooks_default_agent_id: "main".to_owned(),
            openai_chat_completions_enabled: false,
            openresponses_enabled: false,
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
    channels_inbound_token: Option<String>,
    telegram_webhook_secret: Option<String>,
    telegram_bot_token: Option<String>,
    telegram_api_base_url: Option<String>,
    discord_webhook_token: Option<String>,
    discord_outbound_url: Option<String>,
    discord_outbound_token: Option<String>,
    slack_webhook_token: Option<String>,
    slack_outbound_url: Option<String>,
    slack_outbound_token: Option<String>,
    signal_webhook_token: Option<String>,
    signal_outbound_url: Option<String>,
    signal_outbound_token: Option<String>,
    whatsapp_webhook_token: Option<String>,
    whatsapp_outbound_url: Option<String>,
    whatsapp_outbound_token: Option<String>,
    hooks_enabled: Option<bool>,
    hooks_token: Option<String>,
    hooks_path: Option<String>,
    hooks_max_body_bytes: Option<usize>,
    hooks_allow_request_session_key: Option<bool>,
    hooks_default_session_key: Option<String>,
    hooks_default_agent_id: Option<String>,
    openai_chat_completions_enabled: Option<bool>,
    openresponses_enabled: Option<bool>,
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
        override_option(
            &mut self.channels_inbound_token,
            other.channels_inbound_token,
        );
        override_option(
            &mut self.telegram_webhook_secret,
            other.telegram_webhook_secret,
        );
        override_option(&mut self.telegram_bot_token, other.telegram_bot_token);
        override_option(&mut self.telegram_api_base_url, other.telegram_api_base_url);
        override_option(&mut self.discord_webhook_token, other.discord_webhook_token);
        override_option(&mut self.discord_outbound_url, other.discord_outbound_url);
        override_option(
            &mut self.discord_outbound_token,
            other.discord_outbound_token,
        );
        override_option(&mut self.slack_webhook_token, other.slack_webhook_token);
        override_option(&mut self.slack_outbound_url, other.slack_outbound_url);
        override_option(&mut self.slack_outbound_token, other.slack_outbound_token);
        override_option(&mut self.signal_webhook_token, other.signal_webhook_token);
        override_option(&mut self.signal_outbound_url, other.signal_outbound_url);
        override_option(&mut self.signal_outbound_token, other.signal_outbound_token);
        override_option(
            &mut self.whatsapp_webhook_token,
            other.whatsapp_webhook_token,
        );
        override_option(&mut self.whatsapp_outbound_url, other.whatsapp_outbound_url);
        override_option(
            &mut self.whatsapp_outbound_token,
            other.whatsapp_outbound_token,
        );
        override_option(&mut self.hooks_enabled, other.hooks_enabled);
        override_option(&mut self.hooks_token, other.hooks_token);
        override_option(&mut self.hooks_path, other.hooks_path);
        override_option(&mut self.hooks_max_body_bytes, other.hooks_max_body_bytes);
        override_option(
            &mut self.hooks_allow_request_session_key,
            other.hooks_allow_request_session_key,
        );
        override_option(
            &mut self.hooks_default_session_key,
            other.hooks_default_session_key,
        );
        override_option(
            &mut self.hooks_default_agent_id,
            other.hooks_default_agent_id,
        );
        override_option(
            &mut self.openai_chat_completions_enabled,
            other.openai_chat_completions_enabled,
        );
        override_option(&mut self.openresponses_enabled, other.openresponses_enabled);
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

pub(crate) fn default_static_config_paths() -> Vec<PathBuf> {
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

pub(crate) fn user_config_toml_path() -> Option<PathBuf> {
    let home = home_dir();
    user_config_toml_path_for(home.as_deref())
}

pub(crate) fn user_config_toml_path_for(home: Option<&Path>) -> Option<PathBuf> {
    home.map(|home| home.join(".reclaw/config.toml"))
}

pub(crate) fn system_config_toml_path() -> PathBuf {
    PathBuf::from("/etc/reclaw/config.toml")
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

fn normalize_hooks_path(input: String) -> Result<String, String> {
    let mut path = input.trim().to_owned();
    if path.is_empty() {
        return Ok(DEFAULT_HOOKS_PATH.to_owned());
    }
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    while path.len() > 1 && path.ends_with('/') {
        path.pop();
    }
    if path == "/" {
        return Err("hooks.path may not be '/'".to_owned());
    }
    Ok(path)
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
        load_static_config_from_paths, resolve_auth_mode, system_config_toml_path,
        user_config_toml_path_for,
    };

    fn empty_args() -> Args {
        Args {
            command: None,
            config: None,
            host: None,
            port: None,
            gateway_token: None,
            gateway_password: None,
            channels_inbound_token: None,
            telegram_webhook_secret: None,
            telegram_bot_token: None,
            telegram_api_base_url: None,
            discord_webhook_token: None,
            discord_outbound_url: None,
            discord_outbound_token: None,
            slack_webhook_token: None,
            slack_outbound_url: None,
            slack_outbound_token: None,
            signal_webhook_token: None,
            signal_outbound_url: None,
            signal_outbound_token: None,
            whatsapp_webhook_token: None,
            whatsapp_outbound_url: None,
            whatsapp_outbound_token: None,
            openai_chat_completions_enabled: None,
            openresponses_enabled: None,
            hooks_enabled: None,
            hooks_token: None,
            hooks_path: None,
            hooks_max_body_bytes: None,
            hooks_allow_request_session_key: None,
            hooks_default_session_key: None,
            hooks_default_agent_id: None,
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

    #[test]
    fn runtime_config_supports_http_compat_endpoint_toggles() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let config_path = temp_dir.path().join("config.toml");
        fs::write(
            &config_path,
            "openaiChatCompletionsEnabled = true\nopenresponsesEnabled = true\n",
        )
        .expect("config should write");

        let mut args = empty_args();
        args.config = Some(config_path);
        args.openresponses_enabled = Some(false);

        let runtime = RuntimeConfig::from_args(args).expect("runtime config should build");
        assert!(runtime.openai_chat_completions_enabled);
        assert!(!runtime.openresponses_enabled);
    }

    #[test]
    fn runtime_config_supports_channel_webhook_tokens() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let config_path = temp_dir.path().join("config.toml");
        fs::write(
            &config_path,
            "discordWebhookToken = \"discord-a\"\ndiscordOutboundUrl = \"https://relay.example/discord\"\nslackWebhookToken = \"slack-a\"\nslackOutboundUrl = \"https://relay.example/slack\"\nsignalWebhookToken = \"signal-a\"\nsignalOutboundUrl = \"https://relay.example/signal\"\nwhatsappWebhookToken = \"wa-a\"\nwhatsappOutboundUrl = \"https://relay.example/whatsapp\"\n",
        )
        .expect("config should write");

        let mut args = empty_args();
        args.config = Some(config_path);
        args.slack_webhook_token = Some("slack-cli".to_owned());
        args.signal_outbound_token = Some("signal-cli-token".to_owned());

        let runtime = RuntimeConfig::from_args(args).expect("runtime config should build");
        assert_eq!(runtime.discord_webhook_token.as_deref(), Some("discord-a"));
        assert_eq!(
            runtime.discord_outbound_url.as_deref(),
            Some("https://relay.example/discord")
        );
        assert_eq!(runtime.slack_webhook_token.as_deref(), Some("slack-cli"));
        assert_eq!(
            runtime.slack_outbound_url.as_deref(),
            Some("https://relay.example/slack")
        );
        assert_eq!(runtime.signal_webhook_token.as_deref(), Some("signal-a"));
        assert_eq!(
            runtime.signal_outbound_url.as_deref(),
            Some("https://relay.example/signal")
        );
        assert_eq!(
            runtime.signal_outbound_token.as_deref(),
            Some("signal-cli-token")
        );
        assert_eq!(runtime.whatsapp_webhook_token.as_deref(), Some("wa-a"));
        assert_eq!(
            runtime.whatsapp_outbound_url.as_deref(),
            Some("https://relay.example/whatsapp")
        );
    }

    #[test]
    fn runtime_config_requires_hooks_token_when_enabled() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let config_path = temp_dir.path().join("config.toml");
        fs::write(&config_path, "hooksEnabled = true\n").expect("config should write");

        let mut args = empty_args();
        args.config = Some(config_path);

        let result = RuntimeConfig::from_args(args);
        assert!(result.is_err());
    }

    #[test]
    fn runtime_config_supports_hooks_settings() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let config_path = temp_dir.path().join("config.toml");
        fs::write(
            &config_path,
            "hooksEnabled = true\nhooksToken = \"hooks-static\"\nhooksPath = \"hooks-ingress\"\nhooksMaxBodyBytes = 777\nhooksAllowRequestSessionKey = true\nhooksDefaultSessionKey = \"hook:default\"\nhooksDefaultAgentId = \"default-agent\"\n",
        )
        .expect("config should write");

        let mut args = empty_args();
        args.config = Some(config_path);
        args.hooks_token = Some("hooks-cli".to_owned());

        let runtime = RuntimeConfig::from_args(args).expect("runtime config should build");
        assert!(runtime.hooks_enabled);
        assert_eq!(runtime.hooks_token.as_deref(), Some("hooks-cli"));
        assert_eq!(runtime.hooks_path, "/hooks-ingress");
        assert_eq!(runtime.hooks_max_body_bytes, 777);
        assert!(runtime.hooks_allow_request_session_key);
        assert_eq!(
            runtime.hooks_default_session_key.as_deref(),
            Some("hook:default")
        );
        assert_eq!(runtime.hooks_default_agent_id, "default-agent");
    }

    #[test]
    fn user_config_path_resolves_with_explicit_home() {
        let home = std::path::Path::new("/home/reclaw");
        let path = user_config_toml_path_for(Some(home)).expect("user config path should resolve");

        assert_eq!(path, home.join(".reclaw/config.toml"));
    }

    #[test]
    fn system_config_path_is_stable() {
        assert_eq!(
            system_config_toml_path(),
            std::path::PathBuf::from("/etc/reclaw/config.toml")
        );
    }
}
