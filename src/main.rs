use clap::Parser;
use reclaw_server::application::{config::Args, startup};
use tracing::error;

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if let Err(error) = startup::run(args).await {
        error!("server failed: {error}");
        std::process::exit(1);
    }
}
