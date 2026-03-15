use anyhow::Result;
use clap::Parser;

fn build_method() -> &'static str {
    if cfg!(debug_assertions) {
        "debug profile"
    } else {
        "release profile"
    }
}

fn log_startup_header() {
    let exe_path = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    tracing::info!("========================================");
    tracing::info!("Age Inbox startup");
    tracing::info!("Executable: {} ({})", env!("CARGO_BIN_NAME"), exe_path);
    tracing::info!(
        "System: {}/{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    tracing::info!("Compilation method: {}", build_method());
    tracing::info!("App version: {}", env!("CARGO_PKG_VERSION"));
    tracing::info!("========================================");
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    log_startup_header();

    let args = age_inbox_cli::cli::Args::parse();

    age_inbox_cli::cli::run_cli(args).await
}
