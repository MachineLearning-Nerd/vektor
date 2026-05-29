use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::error::{Result, VektorError};

#[derive(Parser, Debug)]
#[command(name = "vektor", version, about, long_about = None)]
pub struct Cli {
    /// Path to config file (default: ~/.vektor/config.toml)
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    /// Increase verbosity (-v for INFO, -vv for DEBUG, -vvv for TRACE)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Index a codebase for later search and context assembly
    Index(IndexArgs),

    /// Start the MCP server for AI coding agents
    Serve(ServeArgs),

    /// Manage local ONNX models
    Models {
        #[command(subcommand)]
        action: ModelsAction,
    },
}

#[derive(clap::Args, Debug)]
pub struct IndexArgs {
    /// Path to the codebase to index
    pub path: PathBuf,

    /// Force a full re-index, ignoring hashes
    #[arg(long)]
    pub force: bool,

    /// Print chunks instead of indexing (debug aid for Phase 2)
    #[arg(long)]
    pub dump_chunks: bool,
}

#[derive(clap::Args, Debug)]
pub struct ServeArgs {
    /// Transport mode: stdio or sse
    #[arg(long)]
    pub transport: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum ModelsAction {
    /// Download the configured ONNX model
    Download {
        /// Download the smaller fallback model
        #[arg(long)]
        lite: bool,
    },
}

pub async fn run(cli: Cli) -> Result<()> {
    let config = crate::config::Config::load(cli.config.clone())?;
    tracing::debug!(?config, "configuration loaded");

    match cli.command {
        Command::Index(args) => {
            tracing::info!(
                path = %args.path.display(),
                force = args.force,
                dump_chunks = args.dump_chunks,
                "vektor index requested"
            );
            Err(VektorError::NotImplemented("vektor index (Phase 2)"))
        }
        Command::Serve(args) => {
            let transport = args.transport.unwrap_or_else(|| config.server.mode.clone());
            tracing::info!(%transport, "vektor serve requested");

            match transport.as_str() {
                "stdio" => crate::mcp::start_stdio_server().await,
                "sse" => Err(VektorError::NotImplemented(
                    "vektor serve --transport sse (Phase 4)",
                )),
                _ => Err(VektorError::Config(format!(
                    "unsupported server transport: {transport}"
                ))),
            }
        }
        Command::Models {
            action: ModelsAction::Download { lite },
        } => {
            tracing::info!(lite, "vektor models download requested");
            Err(VektorError::NotImplemented(
                "vektor models download (Phase 3)",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_help_lists_phase_1_subcommands() {
        let help = Cli::command().render_long_help().to_string();

        assert!(help.contains("index"));
        assert!(help.contains("serve"));
        assert!(help.contains("models"));
    }

    #[test]
    fn serve_transport_has_no_cli_default() {
        let help = Cli::command()
            .find_subcommand_mut("serve")
            .expect("serve command exists")
            .render_long_help()
            .to_string();

        assert!(help.contains("--transport"));
        assert!(!help.contains("[default: stdio]"));
    }

    #[test]
    fn global_flags_parse_after_subcommand() {
        let cli = Cli::parse_from([
            "vektor",
            "index",
            "/tmp",
            "--config",
            "/tmp/config.toml",
            "-v",
        ]);

        assert_eq!(cli.config, Some(PathBuf::from("/tmp/config.toml")));
        assert_eq!(cli.verbose, 1);
    }
}
