use anyhow::Result;
use clap::Parser;

mod chunker;
mod cli;
mod config;
mod context;
mod discovery;
mod embedder;
mod error;
mod index_status;
mod mcp;
mod models;
mod search;
mod secrets;
mod shallow_indexer;
mod state;
mod telemetry;
mod text_index;
mod vector_store;
mod warmup;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    let format = match &cli.command {
        cli::Command::Serve(_) => telemetry::Format::Json,
        _ => telemetry::Format::Pretty,
    };

    telemetry::init(cli.verbose, format);
    cli::run(cli).await?;
    Ok(())
}
