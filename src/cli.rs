use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use clap::{Parser, Subcommand};

use crate::{
    chunker::{Chunk, Language, chunk_file},
    config::Config,
    discovery::discover_files,
    embedder::{Embedder, build_embedder},
    error::{Result, VektorError},
    secrets::SecretDetector,
    state::{FileStatus, HashStore, hash_file},
    text_index::TextIndex,
    vector_store::VectorStore,
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
            run_index(args, &config).await
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
            let spec = if lite {
                &crate::models::BGE_SMALL_EN_V1_5
            } else {
                &crate::models::JINA_V2_BASE_CODE
            };
            crate::models::download_model(spec, &config, "https://huggingface.co").await
        }
    }
}

async fn run_index(args: IndexArgs, config: &Config) -> Result<()> {
    // `--dump-chunks` is a read-only debug aid: it must NEVER build the embedder
    // or the vector store (no state, no LanceDB writes). Handle it before the
    // shared index core so that contract holds.
    if args.dump_chunks {
        let (_root, files) = collect_index_files(&args.path, config)?;
        dump_chunks(&files, config)?;
        return Ok(());
    }

    let stats = index_path(&args.path, config, args.force).await?;

    println!("files: {}", stats.files);
    println!("changed: {}", stats.changed);
    println!("unchanged: {}", stats.unchanged);
    println!("failed: {}", stats.failed);
    println!("chunks: {}", stats.chunks);
    println!("embeddings: {}", stats.embeddings);
    println!("reused: {}", stats.reused);
    println!("skipped_secrets: {}", stats.skipped_secrets);

    Ok(())
}

/// Shared async index core used by BOTH the `vektor index` CLI and the
/// `index_codebase` MCP handler.
///
/// It builds the embedder ONCE ([`build_embedder`]) and the [`VectorStore`] ONCE
/// (sized to the embedder's `dim`/`name`), then delegates to
/// [`index_path_with_embedder`], which owns the per-file orchestration loop. The
/// two entry points are thin wrappers: the CLI prints the returned [`IndexStats`]
/// as a human summary, the MCP handler serializes them to JSON. There is no
/// duplicated indexing loop.
///
/// `force` corresponds to the CLI `--force` / MCP `force_full` flag: when `true`,
/// every discovered file is re-chunked/re-indexed regardless of its stored hash
/// (cached vectors are still reused per-chunk inside [`VectorStore::reindex_file`]).
pub(crate) async fn index_path(path: &Path, config: &Config, force: bool) -> Result<IndexStats> {
    index_path_with_options(
        path,
        config,
        IndexOptions {
            force,
            extensions: None,
        },
    )
    .await
}

pub(crate) async fn index_path_with_options(
    path: &Path,
    config: &Config,
    options: IndexOptions,
) -> Result<IndexStats> {
    let (root, files) = collect_index_files_with_options(path, config, &options)?;

    let embedder = build_embedder(config)?;
    let store = VectorStore::new(&root, config, embedder.dim(), embedder.name()).await?;
    let mut text_index = TextIndex::new(&root, config)?;

    index_path_with_embedder(
        &root,
        files,
        config,
        embedder.as_ref(),
        store,
        &mut text_index,
        options.force,
    )
    .await
}

#[derive(Debug, Clone, Default)]
pub(crate) struct IndexOptions {
    pub(crate) force: bool,
    pub(crate) extensions: Option<Vec<String>>,
}

pub(crate) trait TextIndexWriter: Send {
    fn delete_by_file(&mut self, rel_path: &str) -> Result<()>;
    fn add_chunks(&mut self, chunks: &[Chunk]) -> Result<()>;
    fn commit(&mut self) -> Result<()>;
}

impl TextIndexWriter for TextIndex {
    fn delete_by_file(&mut self, rel_path: &str) -> Result<()> {
        TextIndex::delete_by_file(self, rel_path)
    }

    fn add_chunks(&mut self, chunks: &[Chunk]) -> Result<()> {
        TextIndex::add_chunks(self, chunks)
    }

    fn commit(&mut self) -> Result<()> {
        TextIndex::commit(self)
    }
}

/// Per-file index orchestration over an already-built embedder + store.
///
/// Split out from [`index_path`] as a TEST SEAM: tests inject a fake embedder and
/// a tempdir-backed store so the full pipeline runs in CI without downloading the
/// real ONNX model. Production callers go through [`index_path`].
///
/// Preserves the Phase 2 contract:
/// - `should_skip_file` runs BEFORE any bytes are read (file-level secret gate);
/// - `HashStore` status order is `Pending` before processing, then `Indexed`
///   only after the final Tantivy commit succeeds or `Failed` on per-file error;
/// - chunks whose content trips `contains_secret` are dropped BEFORE embedding so
///   secrets never reach the embedder, vector store, or Tantivy.
///
/// `pub(crate)`: the MCP handler's integration test drives this directly with a
/// fake embedder + tempdir store to exercise the full tool flow without a model.
pub(crate) async fn index_path_with_embedder(
    root: &Path,
    files: Vec<IndexFile>,
    config: &Config,
    embedder: &dyn Embedder,
    mut store: VectorStore,
    text_index: &mut dyn TextIndexWriter,
    force: bool,
) -> Result<IndexStats> {
    let hash_store = HashStore::open(root, config)?;
    let detector = SecretDetector::new();
    let mut stats = IndexStats::default();
    let mut processed_files = Vec::new();

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
        let stored_hash = hash_store.get_hash(&file.rel_path)?;
        let stored_status = hash_store.get_status(&file.rel_path)?;
        let should_process = force
            || stored_hash.as_deref() != Some(current_hash.as_str())
            || stored_status != Some(FileStatus::Indexed);

        if !should_process {
            stats.unchanged += 1;
            continue;
        }

        stats.changed += 1;
        hash_store.set_hash(&file.rel_path, &current_hash, FileStatus::Pending)?;

        let content = match read_file_lossy(&file.path) {
            Ok(content) => content,
            Err(error) => {
                stats.failed += 1;
                hash_store.set_hash(&file.rel_path, &current_hash, FileStatus::Failed)?;
                tracing::debug!(
                    path = %file.path.display(),
                    error = %error,
                    "failed to read file during index"
                );
                continue;
            }
        };

        // ── Content-level secret skip (after chunking, BEFORE embedding) ──────
        // Drop any chunk whose content looks like a secret so it is never sent to
        // the embedder or written to the vector store.
        let all_chunks = chunk_file(Path::new(&file.rel_path), &content, config);
        let mut safe_chunks: Vec<Chunk> = Vec::with_capacity(all_chunks.len());
        for chunk in all_chunks {
            if detector.contains_secret(&chunk.content) {
                stats.skipped_secrets += 1;
                tracing::warn!(
                    path = %file.rel_path,
                    chunk_id = %chunk.id,
                    "skipping chunk with potential secret (not embedded)"
                );
            } else {
                safe_chunks.push(chunk);
            }
        }

        // Delete-then-insert this file's rows; reuse unchanged chunk embeddings.
        let last_modified = file_mtime_secs(&file.path);
        let reindex = match store
            .reindex_file(&file.rel_path, &safe_chunks, embedder, last_modified)
            .await
        {
            Ok(reindex) => reindex,
            Err(error) => {
                stats.failed += 1;
                hash_store.set_hash(&file.rel_path, &current_hash, FileStatus::Failed)?;
                tracing::debug!(
                    path = %file.path.display(),
                    error = %error,
                    "failed to write vector rows during index"
                );
                continue;
            }
        };

        if let Err(error) = text_index
            .delete_by_file(&file.rel_path)
            .and_then(|_| text_index.add_chunks(&safe_chunks))
        {
            if let Err(cleanup_error) = text_index.delete_by_file(&file.rel_path) {
                tracing::debug!(
                    path = %file.path.display(),
                    error = %cleanup_error,
                    "failed to queue Tantivy cleanup after per-file write failure"
                );
            }
            stats.failed += 1;
            hash_store.set_hash(&file.rel_path, &current_hash, FileStatus::Failed)?;
            tracing::debug!(
                path = %file.path.display(),
                error = %error,
                "failed to write Tantivy rows during index"
            );
            continue;
        }

        stats.chunks += reindex.chunks;
        stats.embeddings += reindex.embedded;
        stats.reused += reindex.reused;

        // M-3 decision: a file whose chunks were ALL secret-dropped is still marked
        // Indexed (it was processed, has 0 vectors, won't reprocess until its hash
        // changes). Keep it Pending until the final Tantivy commit succeeds.
        processed_files.push((file.rel_path, current_hash));
    }

    text_index.commit()?;

    for (rel_path, hash) in processed_files {
        hash_store.set_hash(&rel_path, &hash, FileStatus::Indexed)?;
    }

    Ok(stats)
}

fn filter_index_files(files: Vec<IndexFile>, extensions: Option<&[String]>) -> Vec<IndexFile> {
    let Some(extensions) = extensions else {
        return files;
    };

    files
        .into_iter()
        .filter(|file| file_matches_extensions(&file.rel_path, extensions))
        .collect()
}

fn file_matches_extensions(rel_path: &str, extensions: &[String]) -> bool {
    let Some(extension) = Path::new(rel_path).extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    let extension = extension.to_ascii_lowercase();

    extensions.iter().any(|allowed| allowed == &extension)
}

/// File modification time as Unix seconds; falls back to `0` if unavailable
/// (e.g. a platform without mtime). `last_modified` only feeds recency ranking,
/// so a missing value degrades gracefully rather than failing the index.
fn file_mtime_secs(path: &Path) -> i64 {
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
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
            if detector.contains_secret(&chunk.content) {
                tracing::warn!(
                    path = %file.rel_path,
                    chunk_id = %chunk.id,
                    "dump_chunks: skipping chunk with potential secret"
                );
                continue;
            }
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

pub(crate) fn collect_index_files(
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

pub(crate) fn collect_index_files_with_options(
    input: &Path,
    config: &crate::config::Config,
    options: &IndexOptions,
) -> Result<(PathBuf, Vec<IndexFile>)> {
    let (root, files) = collect_index_files(input, config)?;
    Ok((
        root,
        filter_index_files(files, options.extensions.as_deref()),
    ))
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
pub(crate) struct IndexFile {
    path: PathBuf,
    pub(crate) rel_path: String,
}

/// Run-level index outcome aggregated across all files.
///
/// `pub(crate)` so the MCP `index_codebase` handler can read these fields and
/// serialize them to JSON (the CLI prints them as a human summary).
#[derive(Debug, Default)]
pub(crate) struct IndexStats {
    pub files: usize,
    pub changed: usize,
    pub unchanged: usize,
    pub failed: usize,
    pub chunks: usize,
    /// Chunks freshly embedded across all files (cache misses).
    pub embeddings: usize,
    /// Chunks whose vector was reused from the existing-embeddings cache.
    pub reused: usize,
    /// Number of files and chunks skipped because they matched secret patterns.
    /// File-level skips are detected before bytes are read; chunk-level skips
    /// occur after chunking but before embedding.
    pub skipped_secrets: usize,
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

    // -----------------------------------------------------------------------
    // 3.7c — shared index core tests (fake-embedder seam)
    //
    // These exercise `index_path_with_embedder` end-to-end (discover → secret
    // skip → reindex_file → LanceDB) with a fake embedder so CI needs no model
    // download. They share the exact pipeline the CLI and MCP handler use.
    // -----------------------------------------------------------------------

    use crate::embedder::Embedder;
    use crate::vector_store::VectorStore;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TEST_DIM: usize = 8;
    const TEST_MODEL: &str = "fake-embedder";

    /// Fake embedder counting embedded texts; returns deterministic vectors so the
    /// content-hash reuse path behaves like the real one (identical content ⇒
    /// identical vector).
    struct FakeEmbedder {
        texts: AtomicUsize,
    }

    impl FakeEmbedder {
        fn new() -> Self {
            Self {
                texts: AtomicUsize::new(0),
            }
        }
        fn texts_embedded(&self) -> usize {
            self.texts.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl Embedder for FakeEmbedder {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.texts.fetch_add(texts.len(), Ordering::SeqCst);
            Ok(texts
                .iter()
                .map(|t| vec![t.len() as f32; TEST_DIM])
                .collect())
        }
        fn dim(&self) -> usize {
            TEST_DIM
        }
        fn name(&self) -> &str {
            TEST_MODEL
        }
        fn prefix_for_document(&self) -> &str {
            ""
        }
        fn prefix_for_query(&self) -> &str {
            ""
        }
    }

    /// Config whose `data_dir` is a sibling of the indexed dir, so state.db and
    /// the LanceDB `lance/` directory are NOT written inside the tree being
    /// indexed (which would make them show up as files on the next run).
    fn config_for(data_dir: &Path) -> Config {
        let mut config = Config::default();
        config.index.data_dir = data_dir.to_string_lossy().into_owned();
        config
    }

    /// Run the shared core against `root` with a freshly built fake embedder + store.
    async fn index_with_fake(root: &Path, config: &Config, force: bool) -> (IndexStats, usize) {
        let (collected_root, files) = collect_index_files(root, config).expect("collect files");
        let embedder = FakeEmbedder::new();
        let store = VectorStore::new(&collected_root, config, TEST_DIM, TEST_MODEL)
            .await
            .expect("create store");
        let mut text_index =
            crate::text_index::TextIndex::new(&collected_root, config).expect("create text index");
        let stats = index_path_with_embedder(
            &collected_root,
            files,
            config,
            &embedder,
            store,
            &mut text_index,
            force,
        )
        .await
        .expect("index");
        (stats, embedder.texts_embedded())
    }

    fn tantivy_docs_for_path(root: &Path, config: &Config, rel_path: &str) -> usize {
        let index = crate::text_index::TextIndex::new(root, config).expect("open text index");
        let searcher = index.reader().searcher();
        let term = tantivy::Term::from_field_text(index.fields().rel_path, rel_path);
        let query = tantivy::query::TermQuery::new(term, tantivy::schema::IndexRecordOption::Basic);

        searcher
            .search(&query, &tantivy::collector::Count)
            .expect("count Tantivy docs")
    }

    #[tokio::test]
    async fn index_cli_second_run_unchanged_embeds_nothing() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let dir = tempdir.path().join("repo");
        std::fs::create_dir_all(&dir).expect("mkdir repo");
        std::fs::write(dir.join("lib.rs"), "pub fn hello() -> u32 { 1 }\n").expect("write");
        let config = config_for(&tempdir.path().join("data"));
        let dir = dir.as_path();

        let (first, first_embeds) = index_with_fake(dir, &config, false).await;
        assert!(first.chunks > 0, "first run produces chunks");
        assert!(first_embeds > 0, "first run embeds chunks");
        assert_eq!(first.embeddings, first_embeds);
        assert_eq!(first.reused, 0, "nothing to reuse on first run");

        let (second, second_embeds) = index_with_fake(dir, &config, false).await;
        assert_eq!(second.embeddings, 0, "unchanged second run embeds nothing");
        assert_eq!(second_embeds, 0);
        assert_eq!(second.unchanged, 1, "the one file is unchanged");
        assert_eq!(second.changed, 0);
    }

    #[tokio::test]
    async fn index_cli_changed_file_embeds_only_changed_content() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let dir = tempdir.path().join("repo");
        std::fs::create_dir_all(&dir).expect("mkdir repo");
        let file = dir.join("lib.rs");
        std::fs::write(
            &file,
            "pub fn a() -> u32 { 1 }\n\npub fn b() -> u32 { 2 }\n",
        )
        .expect("write");
        let config = config_for(&tempdir.path().join("data"));
        let dir = dir.as_path();

        let (first, _) = index_with_fake(dir, &config, false).await;
        let first_chunks = first.chunks;
        assert!(first_chunks >= 1);

        // Modify only fn b. fn a's chunk content is unchanged ⇒ its vector reused.
        std::fs::write(
            &file,
            "pub fn a() -> u32 { 1 }\n\npub fn b() -> u32 { 999 }\n",
        )
        .expect("rewrite");

        let (second, second_embeds) = index_with_fake(dir, &config, false).await;
        assert_eq!(second.changed, 1, "the file changed");
        // At least one chunk is reused (fn a) and fewer than all are embedded,
        // proving content-hash reuse works through the real chunker.
        assert!(second.reused >= 1, "fn a's chunk is reused: {second:?}",);
        assert!(
            second.embeddings < second.chunks,
            "not every chunk re-embedded (reuse happened)"
        );
        assert_eq!(second.embeddings, second_embeds);
    }

    #[tokio::test]
    async fn index_cli_writes_tantivy_docs_without_duplicates_on_unchanged_run() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let dir = tempdir.path().join("repo");
        std::fs::create_dir_all(&dir).expect("mkdir repo");
        std::fs::write(dir.join("lib.rs"), "pub fn hello() -> u32 { 1 }\n").expect("write");
        let config = config_for(&tempdir.path().join("data"));
        let dir = dir.as_path();

        let (first, _) = index_with_fake(dir, &config, false).await;
        let first_docs = tantivy_docs_for_path(dir, &config, "lib.rs");
        assert_eq!(first_docs, first.chunks);

        let (second, _) = index_with_fake(dir, &config, false).await;
        let second_docs = tantivy_docs_for_path(dir, &config, "lib.rs");
        assert_eq!(second.changed, 0);
        assert_eq!(
            second_docs, first_docs,
            "unchanged re-index must not duplicate Tantivy docs"
        );
    }

    #[tokio::test]
    async fn index_cli_changed_file_updates_only_that_files_tantivy_docs() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let dir = tempdir.path().join("repo");
        std::fs::create_dir_all(&dir).expect("mkdir repo");
        let changed_file = dir.join("lib.rs");
        std::fs::write(&changed_file, "pub fn hello() -> u32 { 1 }\n").expect("write lib");
        std::fs::write(dir.join("other.rs"), "pub fn other() -> u32 { 2 }\n").expect("write other");
        let config = config_for(&tempdir.path().join("data"));
        let dir = dir.as_path();

        let (first, _) = index_with_fake(dir, &config, false).await;
        assert_eq!(first.changed, 2);
        let other_docs_before = tantivy_docs_for_path(dir, &config, "other.rs");
        assert!(other_docs_before > 0);

        std::fs::write(&changed_file, "pub fn hello() -> u32 { 999 }\n").expect("rewrite lib");
        let (second, _) = index_with_fake(dir, &config, false).await;

        assert_eq!(second.changed, 1);
        assert_eq!(
            tantivy_docs_for_path(dir, &config, "lib.rs"),
            second.chunks,
            "changed file docs should be replaced with the current chunk set"
        );
        assert_eq!(
            tantivy_docs_for_path(dir, &config, "other.rs"),
            other_docs_before,
            "unchanged file docs should not be deleted or duplicated"
        );
    }

    /// `.env` must be skipped at the file level (never read) AND a secret in a
    /// safe-named source file must be dropped at the chunk level before embedding.
    #[tokio::test]
    async fn index_cli_skips_secret_file_and_secret_chunk() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let dir = tempdir.path().join("repo");
        std::fs::create_dir_all(&dir).expect("mkdir repo");

        std::fs::write(dir.join("lib.rs"), "pub fn hello() -> u32 { 1 }\n").expect("write lib");
        // .env: file-level skip (never read).
        std::fs::write(dir.join(".env"), "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n")
            .expect("write .env");
        // A safe-named .txt file whose content carries a secret → chunk-level drop.
        std::fs::write(
            dir.join("notes.txt"),
            "config notes\nAWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\nmore notes\n",
        )
        .expect("write notes");

        let config = config_for(&tempdir.path().join("data"));
        let dir = dir.as_path();
        let (stats, _) = index_with_fake(dir, &config, false).await;

        assert!(
            stats.skipped_secrets >= 2,
            "both the .env file and the secret chunk are skipped+counted: {stats:?}"
        );

        // The .env file was never processed → no HashStore status.
        let hs = HashStore::open(dir, &config).expect("open hash store");
        assert!(
            hs.get_status(".env").expect("status").is_none(),
            ".env must never be processed"
        );
        assert_eq!(
            tantivy_docs_for_path(dir, &config, "notes.txt"),
            0,
            "secret-bearing chunks must not be written to Tantivy"
        );
    }

    /// `--dump-chunks` must create NO state.db and NO LanceDB store.
    #[tokio::test]
    async fn index_cli_dump_chunks_writes_no_state() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let dir = tempdir.path();
        let data = tempdir.path().join("data");
        std::fs::write(dir.join("lib.rs"), "pub fn hello() -> u32 { 1 }\n").expect("write");

        let mut config = Config::default();
        config.index.data_dir = data.to_string_lossy().into_owned();

        let args = IndexArgs {
            path: dir.to_path_buf(),
            force: false,
            dump_chunks: true,
        };
        run_index(args, &config).await.expect("dump_chunks ok");

        // No per-project data dir should have been created (no state.db, no lance/).
        assert!(
            !data.exists()
                || std::fs::read_dir(&data)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(true),
            "--dump-chunks must not create any project state or vector storage"
        );
    }

    mod index_cli {
        mod tests {
            use super::super::*;

            struct FailingCommitTextIndex {
                added_chunks: usize,
                commits: usize,
            }

            impl FailingCommitTextIndex {
                fn new() -> Self {
                    Self {
                        added_chunks: 0,
                        commits: 0,
                    }
                }
            }

            impl TextIndexWriter for FailingCommitTextIndex {
                fn delete_by_file(&mut self, _rel_path: &str) -> Result<()> {
                    Ok(())
                }

                fn add_chunks(&mut self, chunks: &[Chunk]) -> Result<()> {
                    self.added_chunks += chunks.len();
                    Ok(())
                }

                fn commit(&mut self) -> Result<()> {
                    self.commits += 1;
                    Err(VektorError::Storage(
                        "forced Tantivy commit failure".to_string(),
                    ))
                }
            }

            #[tokio::test]
            async fn tantivy_commit_failure_leaves_files_pending() {
                let tempdir = tempfile::tempdir().expect("tempdir");
                let dir = tempdir.path().join("repo");
                std::fs::create_dir_all(&dir).expect("mkdir repo");
                std::fs::write(dir.join("lib.rs"), "pub fn hello() -> u32 { 1 }\n").expect("write");
                let config = config_for(&tempdir.path().join("data"));
                let (root, files) = collect_index_files(&dir, &config).expect("collect files");
                let embedder = FakeEmbedder::new();
                let store = VectorStore::new(&root, &config, TEST_DIM, TEST_MODEL)
                    .await
                    .expect("create store");
                let mut text_index = FailingCommitTextIndex::new();

                let error = index_path_with_embedder(
                    &root,
                    files,
                    &config,
                    &embedder,
                    store,
                    &mut text_index,
                    false,
                )
                .await
                .expect_err("commit failure should abort the run");

                assert!(
                    error.to_string().contains("forced Tantivy commit failure"),
                    "unexpected error: {error}"
                );
                assert_eq!(text_index.commits, 1);
                assert!(text_index.added_chunks > 0);

                let hash_store = HashStore::open(&dir, &config).expect("open hash store");
                assert_eq!(
                    hash_store.get_status("lib.rs").expect("status"),
                    Some(FileStatus::Pending),
                    "successful per-file writes stay Pending until Tantivy commit succeeds"
                );
            }
        }
    }
}
