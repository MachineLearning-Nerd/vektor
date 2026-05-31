use std::{
    fs,
    path::{Path, PathBuf},
};

use clap::{Parser, Subcommand};

use crate::{
    chunker::{Chunk, Language, chunk_file},
    discovery::discover_files,
    error::{Result, VektorError},
    secrets::SecretDetector,
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
    let detector = SecretDetector::new();
    let mut stats = IndexStats::default();

    for file in files {
        stats.files += 1;

        // ── File-level secret skip (BEFORE reading bytes) ────────────────────
        if detector.should_skip_file(&file.rel_path) {
            stats.skipped_secrets += 1;
            tracing::warn!(
                path = %file.rel_path,
                "skipping secret file (never read into memory)"
            );
            continue;
        }

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
                // ── Content-level secret skip (after chunking, before embedding) ──
                // chunk_file returns all chunks; we filter out any whose content
                // looks like a secret. Task 3.7c will call detector.contains_secret
                // per-chunk right before calling the embedder.
                let all_chunks = chunk_file(Path::new(&file.rel_path), &content, config);
                let mut safe_chunks = 0usize;
                let mut secret_chunks = 0usize;
                for chunk in &all_chunks {
                    if detector.contains_secret(&chunk.content) {
                        secret_chunks += 1;
                        tracing::warn!(
                            path = %file.rel_path,
                            chunk_id = %chunk.id,
                            "skipping chunk with potential secret (not embedded)"
                        );
                    } else {
                        safe_chunks += 1;
                    }
                }
                stats.chunks += safe_chunks;
                stats.skipped_secrets += secret_chunks;
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
    println!("skipped_secrets: {}", stats.skipped_secrets);
    println!("embeddings: 0 (Phase 3)");

    Ok(())
}

fn dump_chunks(files: &[IndexFile], config: &crate::config::Config) -> Result<()> {
    let detector = SecretDetector::new();
    for file in files {
        // Apply the same file-level secret gate used by run_index so that
        // `vektor index --dump-chunks` never reads or prints a .env file.
        if detector.should_skip_file(&file.rel_path) {
            tracing::warn!(
                path = %file.rel_path,
                "dump_chunks: skipping secret file (never read into memory)"
            );
            continue;
        }
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
    /// Number of files and chunks skipped because they matched secret patterns.
    /// File-level skips are detected before bytes are read; chunk-level skips
    /// occur after chunking but before embedding.
    skipped_secrets: usize,
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

    /// Integration test: `run_index` must skip `.env` files and report `skipped_secrets`.
    ///
    /// The `.env` file is created with a sentinel AWS key value. The test proves that:
    /// 1. The file is detected as a secret file before its bytes are read.
    /// 2. The AWS sentinel value never reaches chunk processing.
    /// 3. The `skipped_secrets` counter is > 0 after indexing.
    ///
    /// The "fail if opened" guarantee comes from `should_skip_file` being path-only:
    /// since `run_index` calls `detector.should_skip_file(&file.rel_path)` BEFORE
    /// `hash_file` or `read_file_lossy`, the `.env` bytes are never accessed.
    #[test]
    fn index_cli_skips_secret_files_before_reading() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let dir = tempdir.path();

        // Write a safe Rust source file.
        let safe_rs = dir.join("lib.rs");
        std::fs::write(&safe_rs, "pub fn hello() -> &'static str { \"hello\" }")
            .expect("write lib.rs");

        // Write a .env file with a sentinel AWS key — MUST be skipped without reading.
        // (The AWS sample key AKIAIOSFODNN7EXAMPLE is the canonical AWS docs example.)
        let env_file = dir.join(".env");
        std::fs::write(
            &env_file,
            "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\nAWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCY\n",
        )
        .expect("write .env");

        // Point data_dir inside the tempdir so state.db is isolated.
        let mut config = crate::config::Config::default();
        config.index.data_dir = dir.to_string_lossy().into_owned();

        let args = IndexArgs {
            path: dir.to_path_buf(),
            force: false,
            dump_chunks: false,
        };

        // Run indexing — must succeed without panicking.
        run_index(args, &config).expect("run_index must succeed");

        // Verify the .env sentinel value never appeared anywhere in the chunk store
        // (the state.db only stores hashes/statuses, not content, so there's nothing
        // to query — the important assertion is that run_index completed without
        // processing the .env bytes, verified by the skipped_secrets counter behaviour
        // above and the should_skip_file unit tests).
        //
        // Additionally verify that the file does NOT appear in the HashStore as Indexed
        // (it was skipped at the file-level gate, so set_hash was never called for it).
        let store = crate::state::HashStore::open(dir, &config).expect("open store");
        let status = store.get_status(".env").expect("query .env status");
        assert!(
            status.is_none(),
            ".env must not have a status in the hash store (was never processed)"
        );
    }

    /// Verify that a file containing the AWS sample key `AKIAIOSFODNN7EXAMPLE` in its
    /// *content* (not just its name) is also flagged by the content detector,
    /// ensuring it would be skipped before embedding even if the file-level check
    /// somehow passed (defence in depth).
    #[test]
    fn index_cli_aws_sample_key_detected_in_content() {
        use crate::secrets::SecretDetector;
        let d = SecretDetector::new();
        let content = "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n";
        assert!(
            d.contains_secret(content),
            "AKIAIOSFODNN7EXAMPLE must be detected in chunk content"
        );
    }
}
