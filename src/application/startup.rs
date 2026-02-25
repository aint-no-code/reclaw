use std::future::Future;

use tokio::net::TcpListener;
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, fmt};

use crate::{
    application::{
        config::{Args, RuntimeConfig},
        state::SharedState,
    },
    domain::error::DomainError,
    interfaces::http,
    rpc::methods::{known_events, known_methods},
};

pub async fn run(args: Args) -> Result<(), DomainError> {
    let config = RuntimeConfig::from_args(args)
        .map_err(|error| DomainError::InvalidRequest(format!("configuration error: {error}")))?;

    init_logging(&config.log_filter, config.json_logs)?;
    let listener = TcpListener::bind(config.bind_addr())
        .await
        .map_err(|error| DomainError::Unavailable(format!("failed to bind listener: {error}")))?;

    let signal = shutdown_signal();
    run_with_listener(listener, config, signal).await
}

pub async fn run_with_listener(
    listener: TcpListener,
    config: RuntimeConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), DomainError> {
    info!(
        "starting reclaw-server host={} port={} auth_mode={}",
        config.host,
        config.port,
        config.auth_mode.label()
    );

    let state = SharedState::new(config, known_methods(), known_events()).await?;
    let cron_task = spawn_cron_scheduler(state.clone());
    let serve_result = http::serve(listener, state, shutdown).await;

    if let Some(task) = cron_task {
        task.abort();
        if let Err(error) = task.await {
            warn!("cron scheduler task aborted: {error}");
        }
    }

    serve_result
}

fn init_logging(filter: &str, json_logs: bool) -> Result<(), DomainError> {
    let env_filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = fmt().with_env_filter(env_filter).with_target(false);

    if json_logs {
        builder.json().try_init().map_err(|error| {
            DomainError::Unavailable(format!("failed to initialize logger: {error}"))
        })?;
    } else {
        builder.compact().try_init().map_err(|error| {
            DomainError::Unavailable(format!("failed to initialize logger: {error}"))
        })?;
    }

    Ok(())
}

fn spawn_cron_scheduler(state: SharedState) -> Option<tokio::task::JoinHandle<()>> {
    if !state.config().cron_enabled {
        info!("cron scheduler disabled by runtime config");
        return None;
    }

    let poll_interval = state.config().cron_poll_interval;
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(poll_interval);
        loop {
            ticker.tick().await;
            if let Err(error) = state.tick_cron_jobs().await {
                error!("cron tick failed: {error}");
            }
        }
    }))
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!("shutdown signal received");
}
