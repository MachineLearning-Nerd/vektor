use anyhow::Result;
use clap::Parser;

mod chunker;
mod cli;
mod config;
mod discovery;
mod error;
mod mcp;
mod secrets;
mod state;
mod telemetry;

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
