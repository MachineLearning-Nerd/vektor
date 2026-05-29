use std::{
    fs,
    path::{Path, PathBuf},
};

use clap::{Parser, Subcommand};

use crate::{
    chunker::{Chunk, Language, chunk_file},
    discovery::discover_files,
    error::{Result, VektorError},
    state::{FileStatus, HashStore, hash_file},
};

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
            run_index(args, &config)
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

fn run_index(args: IndexArgs, config: &crate::config::Config) -> Result<()> {
    let (root, files) = collect_index_files(&args.path, config)?;
    if args.dump_chunks {
        dump_chunks(&files, config)?;
        return Ok(());
    }

    let store = HashStore::open(&root, config)?;
    let mut stats = IndexStats::default();

    for file in files {
        stats.files += 1;
        let current_hash = hash_file(&file.path)?;
        let stored_hash = store.get_hash(&file.rel_path)?;
        let stored_status = store.get_status(&file.rel_path)?;
        let should_process = args.force
            || stored_hash.as_deref() != Some(current_hash.as_str())
            || stored_status != Some(FileStatus::Indexed);

        if !should_process {
            stats.unchanged += 1;
            continue;
        }

        stats.changed += 1;
        store.set_hash(&file.rel_path, &current_hash, FileStatus::Pending)?;
        match read_file_lossy(&file.path) {
            Ok(content) => {
                let chunks = chunk_file(Path::new(&file.rel_path), &content, config);
                stats.chunks += chunks.len();
                store.set_hash(&file.rel_path, &current_hash, FileStatus::Indexed)?;
            }
            Err(error) => {
                stats.failed += 1;
                store.set_hash(&file.rel_path, &current_hash, FileStatus::Failed)?;
                tracing::debug!(
                    path = %file.path.display(),
                    error = %error,
                    "failed to read file during Phase 2 index"
                );
            }
        }
    }

    println!("files: {}", stats.files);
    println!("changed: {}", stats.changed);
    println!("unchanged: {}", stats.unchanged);
    println!("failed: {}", stats.failed);
    println!("chunks: {}", stats.chunks);
    println!("embeddings: 0 (Phase 3)");

    Ok(())
}

fn dump_chunks(files: &[IndexFile], config: &crate::config::Config) -> Result<()> {
    for file in files {
        let content = read_file_lossy(&file.path)?;
        let chunks = chunk_file(Path::new(&file.rel_path), &content, config);
        for chunk in chunks {
            print_chunk(&chunk);
        }
    }

    Ok(())
}

fn print_chunk(chunk: &Chunk) {
    println!("--- chunk {} ---", chunk.id);
    println!("path: {}", chunk.rel_path);
    println!("lines: {}-{}", chunk.start_line, chunk.end_line);
    println!("language: {}", language_label(chunk.language));
    if let Some(symbol_name) = chunk.symbol_name.as_deref() {
        println!("symbol: {symbol_name}");
    }
    if let Some(symbol_type) = chunk.symbol_type.as_deref() {
        println!("symbol_type: {symbol_type}");
    }
    println!("content_hash: {}", chunk.content_hash);
    println!("content:");
    println!("{}", chunk.content);
}

fn read_file_lossy(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn language_label(language: Option<Language>) -> &'static str {
    language.map(Language::as_str).unwrap_or("unknown")
}

fn collect_index_files(
    input: &Path,
    config: &crate::config::Config,
) -> Result<(PathBuf, Vec<IndexFile>)> {
    if input.is_file() {
        let root = input
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        if !is_within_size_limit(input, config)? {
            return Ok((root, Vec::new()));
        }
        let rel_path = relative_path_string(input, &root)?;
        return Ok((
            root,
            vec![IndexFile {
                path: input.to_path_buf(),
                rel_path,
            }],
        ));
    }

    if input.is_dir() {
        let mut files = Vec::new();
        for path in discover_files(input, config)? {
            files.push(IndexFile {
                rel_path: relative_path_string(&path, input)?,
                path,
            });
        }
        return Ok((input.to_path_buf(), files));
    }

    Err(VektorError::Config(format!(
        "index path does not exist: {}",
        input.display()
    )))
}

fn is_within_size_limit(path: &Path, config: &crate::config::Config) -> Result<bool> {
    let metadata = path.metadata()?;
    let max_size_bytes = config.index.max_file_size_kb.saturating_mul(1024);
    if metadata.len() <= max_size_bytes {
        return Ok(true);
    }

    tracing::debug!(
        path = %path.display(),
        size_bytes = metadata.len(),
        max_file_size_kb = config.index.max_file_size_kb,
        "skipping oversized file"
    );
    Ok(false)
}

fn relative_path_string(path: &Path, root: &Path) -> Result<String> {
    let rel_path = path.strip_prefix(root).map_err(|error| {
        VektorError::Config(format!(
            "discovered path {} is not under root {}: {error}",
            path.display(),
            root.display()
        ))
    })?;

    Ok(normalized_path(rel_path))
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Debug)]
struct IndexFile {
    path: PathBuf,
    rel_path: String,
}

#[derive(Default)]
struct IndexStats {
    files: usize,
    changed: usize,
    unchanged: usize,
    failed: usize,
    chunks: usize,
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

    #[test]
    fn collect_index_files_applies_size_limit_to_direct_file_inputs() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let path = tempdir.path().join("large.rs");
        std::fs::write(&path, "x").expect("write direct file");
        let mut config = crate::config::Config::default();
        config.index.max_file_size_kb = 0;

        let (_root, files) = collect_index_files(&path, &config).expect("collect direct file");

        assert!(files.is_empty());
    }
}
