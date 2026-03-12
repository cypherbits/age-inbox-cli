use anyhow::Result;
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let args = age_inbox_cli::cli::Args::parse();
    
    age_inbox_cli::cli::run_cli(args).await
}
