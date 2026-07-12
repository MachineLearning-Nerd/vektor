//! Embedded LanceDB vector store: connection + chunks-table schema + metadata.
//!
//! This module owns the storage *contract* that later Phase 3 tasks build on:
//! - 3.7a/3.7c (insert / reindex) append `RecordBatch`es to the chunks table,
//! - 3.8 (search) queries the `vector` column,
//! - 3.9 (delete) removes rows by `content_hash` / `rel_path`.
//!
//! It deliberately implements *only* open/create + schema + metadata. There is
//! no insert, search, delete, or ANN-index building here (YAGNI: those tasks
//! trigger index maintenance themselves).
//!
//! ## Arrow version trap
//! The Arrow schema types are taken from `lancedb`'s own re-export
//! (`lancedb::arrow::arrow_schema`). Pulling a separate `arrow-schema` crate
//! risks a version mismatch with the one `lancedb` resolves transitively
//! (currently 58.x), producing confusing "expected `Schema`, found `Schema`"
//! errors. Do not add a direct arrow dependency. See PRD §11.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, Int64Array, RecordBatch, StringArray, UInt32Array,
    types::Float32Type,
};
use futures::TryStreamExt;
use lancedb::{
    Connection,
    arrow::arrow_schema::{DataType, Field, Schema, SchemaRef},
    connect,
    query::{ExecutableQuery, QueryBase, Select},
    table::{CompactionOptions, OptimizeAction},
};
use serde::{Deserialize, Serialize};

use crate::{
    config::Config,
    error::{Result, VektorError},
    state::project_data_dir,
};

/// Name of the chunks table inside the LanceDB connection.
const CHUNKS_TABLE: &str = "chunks";
/// LanceDB data subdirectory, colocated with `state.db` under the project dir.
const LANCE_SUBDIR: &str = "lance";
/// Sidecar metadata file name (sits next to the `lance/` dir).
const META_FILE: &str = "vector_meta.json";
/// Maximum rows per LanceDB `Table::add` call during a `reindex_file` insert.
/// Bounds the size of any single in-memory `RecordBatch` for large files.
const INSERT_BATCH: usize = 500;

fn search_result_projection() -> Select {
    Select::Columns(vec![
        "id".to_string(),
        "content_hash".to_string(),
        "rel_path".to_string(),
        "start_line".to_string(),
        "end_line".to_string(),
        "symbol_name".to_string(),
        "symbol_type".to_string(),
        "language".to_string(),
        "content".to_string(),
        "last_modified".to_string(),
    ])
}

/// Persisted store metadata. Survives reopen and lets us detect a
/// dimension/model change that mandates a full re-index.
///
/// Stored as a sidecar JSON file rather than a LanceDB table so this task
/// stays free of the insert path (writing rows needs `arrow_array`, which
/// 3.7a owns). A small JSON file persists trivially across reopen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreMeta {
    /// Embedding model that produced the vectors in this store.
    pub model_name: String,
    /// Embedding dimension (the `vector` column's fixed size).
    pub embedding_dim: usize,
    /// Unix seconds of the last completed full index, if any.
    pub last_full_index_at: Option<i64>,
    /// `vektor` version that created/last-wrote this store.
    pub vektor_version: String,
    /// Chunk count snapshot at the last ANN index rebuild (3.7/3.8 maintain).
    pub chunks_at_last_ann_rebuild: u64,
    /// Chunks inserted since the last ANN rebuild (3.7 maintains).
    pub chunks_inserted_since: u64,
    /// Chunks deleted since the last ANN rebuild (3.9 maintains).
    pub chunks_deleted_since: u64,
}

impl StoreMeta {
    fn fresh(model_name: &str, dim: usize) -> Self {
        Self {
            model_name: model_name.to_string(),
            embedding_dim: dim,
            last_full_index_at: None,
            vektor_version: env!("CARGO_PKG_VERSION").to_string(),
            chunks_at_last_ann_rebuild: 0,
            chunks_inserted_since: 0,
            chunks_deleted_since: 0,
        }
    }

    fn load(path: &Path) -> Result<Option<Self>> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| VektorError::Storage(format!("corrupt vector-store metadata: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(VektorError::from(e)),
        }
    }

    /// Persist the metadata atomically: write to a sibling temp file, then
    /// `fs::rename` over the target. `rename` is atomic on the same filesystem,
    /// so a crash mid-write leaves the previous `vector_meta.json` intact rather
    /// than a truncated/empty file (a plain `fs::write` could). The temp file is
    /// a sibling (same dir => same filesystem) so the rename can't cross devices.
    fn save(&self, path: &Path) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| VektorError::Storage(format!("failed to serialize metadata: {e}")))?;
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, bytes)?;
        fs::rename(&tmp_path, path)?;
        Ok(())
    }
}

/// One row to insert into the `chunks` table.
///
/// This is the columnar contract shared by the insert path (3.7a/3.7c) and the
/// delete tests here: callers build a slice of these and hand them to
/// [`VectorStore::insert_chunks`]. Field names/types mirror the Arrow schema
/// ([`chunks_schema`]). Note the deliberate differences from
/// [`crate::chunker::Chunk`]:
/// - line numbers are `u32` (the schema's `UInt32`), not `usize`;
/// - `language` is a NON-null `String` — callers map
///   `Chunk.language: Option<Language>` to a concrete string (e.g. `"unknown"`)
///   *before* constructing a `ChunkRow`, because the `language` column rejects
///   nulls. 3.7c carries that mapping (see [`plan_reindex`]).
#[derive(Debug, Clone)]
pub struct ChunkRow {
    pub id: String,
    pub content_hash: String,
    pub vector: Vec<f32>,
    pub rel_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub symbol_name: Option<String>,
    pub symbol_type: Option<String>,
    pub language: String,
    pub content: String,
    pub last_modified: i64,
}

/// Escape a string for safe embedding inside a single-quoted SQL string literal
/// in a LanceDB predicate (e.g. `rel_path = '<escaped>'`).
///
/// LanceDB predicates are SQL-like (DataFusion). The only metacharacter inside a
/// single-quoted literal is the single quote itself, which is escaped by
/// doubling it (`'` -> `''`) per standard SQL. Everything else — double quotes,
/// spaces, backslashes, etc. — is literal inside single quotes and needs no
/// escaping (SQL string literals are not C-style; `\` is not an escape char).
/// Doubling the quote is what prevents both predicate breakage and injection
/// (a path like `a' OR '1'='1` becomes the inert literal `a'' OR ''1''=''1`).
fn escape_sql_string_literal(value: &str) -> String {
    value.replace('\'', "''")
}

/// Embedded LanceDB vector store for a single project.
pub struct VectorStore {
    conn: Connection,
    lance_dir: PathBuf,
    meta_path: PathBuf,
    meta: StoreMeta,
}

impl VectorStore {
    /// Open (or create) the project-scoped vector store.
    ///
    /// The LanceDB data lives at `<data_dir>/<project-hash>/lance/`, colocated
    /// with `state.db` (same project hash as [`crate::state`]). On first open
    /// the `chunks` table and metadata are created; on reopen they are
    /// validated. A change in `dim` (vs the persisted metadata) is rejected
    /// with a re-index-required [`VektorError::Storage`] error, because the
    /// existing `vector` column is a `FixedSizeList` of the old size.
    pub async fn new(
        project_root: &Path,
        config: &Config,
        dim: usize,
        model_name: &str,
    ) -> Result<Self> {
        if dim == 0 {
            return Err(VektorError::Storage(
                "embedding dimension must be non-zero".into(),
            ));
        }

        let project_dir = project_data_dir(project_root, config)?;
        let lance_dir = project_dir.join(LANCE_SUBDIR);
        let meta_path = project_dir.join(META_FILE);
        fs::create_dir_all(&lance_dir)?;

        // Reconcile metadata before touching LanceDB so a dim/model mismatch
        // fails fast with a clear message (no partial table mutation).
        let meta = match StoreMeta::load(&meta_path)? {
            Some(existing) => {
                if existing.embedding_dim != dim {
                    return Err(VektorError::Storage(format!(
                        "embedding dimension changed ({} -> {}); re-index required: \
                         delete {} and run `vektor index` again",
                        existing.embedding_dim,
                        dim,
                        lance_dir.display(),
                    )));
                }
                if existing.model_name != model_name {
                    return Err(VektorError::Storage(format!(
                        "embedding model changed ({} -> {}); re-index required: \
                         delete {} and run `vektor index` again",
                        existing.model_name,
                        model_name,
                        lance_dir.display(),
                    )));
                }
                existing
            }
            None => StoreMeta::fresh(model_name, dim),
        };

        let lance_uri = lance_dir.to_str().ok_or_else(|| {
            VektorError::Storage(format!("non-UTF-8 lance path: {}", lance_dir.display()))
        })?;
        let conn = connect(lance_uri)
            .execute()
            .await
            .map_err(|e| VektorError::Storage(e.to_string()))?;

        let table_names = conn
            .table_names()
            .execute()
            .await
            .map_err(|e| VektorError::Storage(e.to_string()))?;

        if !table_names.iter().any(|name| name == CHUNKS_TABLE) {
            let schema = chunks_schema(dim);
            conn.create_empty_table(CHUNKS_TABLE, schema)
                .execute()
                .await
                .map_err(|e| VektorError::Storage(e.to_string()))?;
        }

        // Persist (or refresh) metadata last, once the table exists.
        meta.save(&meta_path)?;

        Ok(Self {
            conn,
            lance_dir,
            meta_path,
            meta,
        })
    }

    /// Open an existing project-scoped vector store without requiring a live
    /// embedder. Keyword-only search uses this path because it needs stored chunk
    /// content for hydration, but it does not embed the query.
    pub(crate) async fn open_existing(project_root: &Path, config: &Config) -> Result<Self> {
        let project_dir = project_data_dir(project_root, config)?;
        let lance_dir = project_dir.join(LANCE_SUBDIR);
        let meta_path = project_dir.join(META_FILE);
        let meta = StoreMeta::load(&meta_path)?.ok_or_else(|| {
            VektorError::Storage(format!(
                "vector-store metadata not found at {}; run `vektor index` first",
                meta_path.display()
            ))
        })?;

        let lance_uri = lance_dir.to_str().ok_or_else(|| {
            VektorError::Storage(format!("non-UTF-8 lance path: {}", lance_dir.display()))
        })?;
        let conn = connect(lance_uri)
            .execute()
            .await
            .map_err(|e| VektorError::Storage(e.to_string()))?;

        let table_names = conn
            .table_names()
            .execute()
            .await
            .map_err(|e| VektorError::Storage(e.to_string()))?;
        if !table_names.iter().any(|name| name == CHUNKS_TABLE) {
            return Err(VektorError::Storage(format!(
                "chunks table not found in {}; run `vektor index` first",
                lance_dir.display()
            )));
        }

        Ok(Self {
            conn,
            lance_dir,
            meta_path,
            meta,
        })
    }

    pub(crate) async fn open_existing_for_model(
        project_root: &Path,
        config: &Config,
        dim: usize,
        model_name: &str,
    ) -> Result<Self> {
        let store = Self::open_existing(project_root, config).await?;
        if store.meta.embedding_dim != dim {
            return Err(VektorError::Storage(format!(
                "embedding dimension changed ({} -> {}); re-index required: \
                 delete {} and run `vektor index` again",
                store.meta.embedding_dim,
                dim,
                store.lance_dir.display(),
            )));
        }
        if store.meta.model_name != model_name {
            return Err(VektorError::Storage(format!(
                "embedding model changed ({} -> {}); re-index required: \
                 delete {} and run `vektor index` again",
                store.meta.model_name,
                model_name,
                store.lance_dir.display(),
            )));
        }

        Ok(store)
    }

    /// Load persisted vector-store metadata without opening LanceDB.
    pub(crate) fn load_meta(project_root: &Path, config: &Config) -> Result<Option<StoreMeta>> {
        let project_dir = project_data_dir(project_root, config)?;
        StoreMeta::load(&project_dir.join(META_FILE))
    }

    /// Persisted metadata for this store.
    ///
    /// `#[allow(dead_code)]`: read by tests and future search/status tasks
    /// (3.8/3.12); the index path does not need it.
    #[allow(dead_code)]
    pub fn meta(&self) -> &StoreMeta {
        &self.meta
    }

    pub(crate) fn mark_full_index_completed(&mut self, timestamp_secs: i64) -> Result<()> {
        self.meta.last_full_index_at = Some(timestamp_secs);
        self.meta.save(&self.meta_path)
    }

    /// Directory holding the LanceDB dataset (`<project-dir>/lance/`).
    ///
    /// `#[allow(dead_code)]`: read by tests; not needed by the index path.
    #[allow(dead_code)]
    pub fn lance_dir(&self) -> &Path {
        &self.lance_dir
    }

    /// Name of the chunks table.
    ///
    /// `#[allow(dead_code)]`: convenience accessor for future search task (3.8).
    #[allow(dead_code)]
    pub fn chunks_table_name(&self) -> &'static str {
        CHUNKS_TABLE
    }

    /// Open the chunks table (used by insert/search/delete tasks).
    pub async fn chunks_table(&self) -> Result<lancedb::Table> {
        self.conn
            .open_table(CHUNKS_TABLE)
            .execute()
            .await
            .map_err(|e| VektorError::Storage(e.to_string()))
    }

    /// Append rows to the `chunks` table.
    ///
    /// This is the shared insert primitive: it builds a single Arrow
    /// `RecordBatch` matching [`chunks_schema`] from `rows` and `Table::add`s it.
    /// Hoisted to `pub(crate)` so 3.7a/3.7c reuse the exact RecordBatch
    /// construction instead of re-deriving it (3.9 needs it only to seed delete
    /// tests, but the columnar mapping is the same contract).
    ///
    /// Inserting an empty slice is a no-op (`Ok(())`) — no batch, no churn.
    pub(crate) async fn insert_chunks(&mut self, rows: &[ChunkRow]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }

        let dim = self.meta.embedding_dim;
        for row in rows {
            if row.vector.len() != dim {
                return Err(VektorError::Storage(format!(
                    "chunk {} vector has {} dims, store expects {}",
                    row.id,
                    row.vector.len(),
                    dim,
                )));
            }
        }

        let schema = chunks_schema(dim);

        let id = StringArray::from_iter_values(rows.iter().map(|r| r.id.as_str()));
        let content_hash =
            StringArray::from_iter_values(rows.iter().map(|r| r.content_hash.as_str()));
        let vector = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
            rows.iter()
                .map(|r| Some(r.vector.iter().map(|&f| Some(f)).collect::<Vec<_>>())),
            dim as i32,
        );
        let rel_path = StringArray::from_iter_values(rows.iter().map(|r| r.rel_path.as_str()));
        let start_line = UInt32Array::from_iter_values(rows.iter().map(|r| r.start_line));
        let end_line = UInt32Array::from_iter_values(rows.iter().map(|r| r.end_line));
        let symbol_name: StringArray = rows.iter().map(|r| r.symbol_name.as_deref()).collect();
        let symbol_type: StringArray = rows.iter().map(|r| r.symbol_type.as_deref()).collect();
        let language = StringArray::from_iter_values(rows.iter().map(|r| r.language.as_str()));
        let content = StringArray::from_iter_values(rows.iter().map(|r| r.content.as_str()));
        let last_modified = Int64Array::from_iter_values(rows.iter().map(|r| r.last_modified));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(id),
                Arc::new(content_hash),
                Arc::new(vector),
                Arc::new(rel_path),
                Arc::new(start_line),
                Arc::new(end_line),
                Arc::new(symbol_name),
                Arc::new(symbol_type),
                Arc::new(language),
                Arc::new(content),
                Arc::new(last_modified),
            ],
        )
        .map_err(|e| VektorError::Storage(format!("failed to build chunk record batch: {e}")))?;

        let table = self.chunks_table().await?;
        table
            .add(batch)
            .execute()
            .await
            .map_err(|e| VektorError::Storage(e.to_string()))?;

        self.meta.chunks_inserted_since = self
            .meta
            .chunks_inserted_since
            .saturating_add(rows.len() as u64);
        self.meta.save(&self.meta_path)?;

        Ok(())
    }

    /// Load the existing embeddings for ONE file, keyed by `content_hash`.
    ///
    /// This is the read half of read-before-delete re-indexing (PRD §4.5): 3.7b
    /// calls it *before* [`VectorStore::delete_by_file`] so unchanged chunks can
    /// reuse their stored vector instead of being re-embedded. The cache key is
    /// `content_hash` because content reuse is the contract — identical content
    /// hashes to the same value and must embed to the same vector.
    ///
    /// The query is filtered by EXACT `rel_path` (via [`escape_sql_string_literal`],
    /// same predicate as [`VectorStore::delete_by_file`]) so it touches only this
    /// file's rows, never a full-table scan. Only the `content_hash` and `vector`
    /// columns are selected — the columnar store reads exactly those two columns.
    ///
    /// Behavior:
    /// - A missing file (no matching rows) returns an EMPTY map, not an error.
    /// - If the same `content_hash` appears on more than one row of this file
    ///   (duplicate co-located chunks with identical content), ONE vector is kept
    ///   (first wins) and the collision is logged at `debug`. Identical content ⇒
    ///   identical embedding, so the choice is immaterial.
    ///
    /// SIDE-EFFECT FREE: takes `&self` and only reads — it never deletes or
    /// mutates rows or metadata, which is what makes it safe to call *before*
    /// `delete_by_file`. This is also the result-`RecordBatch` column-extraction
    /// pattern that 3.8 (search) reuses to read columns back out.
    ///
    /// Single-file convenience over [`Self::existing_embeddings_for_files`];
    /// production index runs use the batched form directly (test-only today,
    /// retained for the future single-file watcher path).
    #[allow(dead_code)]
    pub(crate) async fn existing_embeddings_by_content_hash(
        &self,
        rel_path: &str,
    ) -> Result<HashMap<String, Vec<f32>>> {
        self.existing_embeddings_for_files(&[rel_path]).await
    }

    /// Batched variant of [`Self::existing_embeddings_by_content_hash`]: load
    /// the existing embeddings for MANY files in a single query, flattened into
    /// one `content_hash → vector` map.
    ///
    /// Flattening across files is sound because reuse is content-addressed:
    /// identical content hashes to the same value and must embed to the same
    /// vector, regardless of which file it lives in. This also lets a batch
    /// reuse a vector across files (moved/duplicated code embeds zero times).
    ///
    /// An empty `rel_paths` returns an empty map without querying.
    pub(crate) async fn existing_embeddings_for_files(
        &self,
        rel_paths: &[&str],
    ) -> Result<HashMap<String, Vec<f32>>> {
        if rel_paths.is_empty() {
            return Ok(HashMap::new());
        }

        let table = self.chunks_table().await?;
        let stream = table
            .query()
            .only_if(rel_paths_in_predicate(rel_paths))
            .select(Select::Columns(vec![
                "content_hash".to_string(),
                "vector".to_string(),
            ]))
            .execute()
            .await
            .map_err(|e| VektorError::Storage(e.to_string()))?;

        let batches: Vec<RecordBatch> = stream
            .try_collect()
            .await
            .map_err(|e| VektorError::Storage(e.to_string()))?;

        hash_vector_map_from_batches(&batches)
    }

    /// Delete every chunk row whose `rel_path` exactly equals `rel_path`.
    ///
    /// Matching is EXACT (`rel_path = '<escaped>'`), never prefix/glob/substring,
    /// so deleting `src/a.rs` leaves `src/a.rs.bak` and `src/a/b.rs` untouched.
    /// The path is escaped via [`escape_sql_string_literal`] before being spliced
    /// into the predicate, so paths containing `'` (and trivially `"`, spaces,
    /// backslashes) are handled safely and cannot break or inject the predicate.
    ///
    /// Deleting a path with no matching rows is SUCCESS, returning `0`. When rows
    /// are removed, `chunks_deleted_since` is incremented by the deleted count and
    /// the metadata is persisted (so ANN-rebuild churn logic can use it).
    ///
    /// Returns the number of rows deleted (from LanceDB's `DeleteResult`).
    ///
    /// Single-file convenience over [`Self::delete_by_files`]; production
    /// index runs use the batched form directly (test-only today, retained
    /// for the future single-file watcher path).
    #[allow(dead_code)]
    pub(crate) async fn delete_by_file(&mut self, rel_path: &str) -> Result<usize> {
        self.delete_by_files(&[rel_path]).await
    }

    /// Re-index a single file: refresh ALL of its rows to match `chunks`.
    ///
    /// This is the per-file integration crux of Phase 3 (PRD §4.5 delete-then-insert
    /// re-indexing). It is the ONE place the CLI and MCP handler funnel through, so
    /// the read/delete/embed/insert ordering lives here exactly once.
    ///
    /// ## Order (must not be reordered)
    /// 1. **Read cache FIRST** ([`Self::existing_embeddings_by_content_hash`]) — the
    ///    side-effect-free read of this file's current vectors keyed by
    ///    `content_hash`. It MUST run before the delete, or there would be nothing
    ///    left to reuse.
    /// 2. **Delete** ([`Self::delete_by_file`]) the file's existing rows so a
    ///    re-index never leaves orphaned/duplicate chunks (delete-then-insert).
    /// 3. **Plan** ([`plan_reindex`]) — diff `chunks` against the cache, reusing
    ///    cached vectors for unchanged content and embedding only the misses in a
    ///    single `embed_documents` call.
    /// 4. **Insert** the prepared rows via [`Self::insert_chunks`] in batches of at
    ///    most [`INSERT_BATCH`] rows (LanceDB appends one `RecordBatch` per call;
    ///    chunking keeps each batch bounded for large files).
    ///
    /// ## Empty `chunks`
    /// The old rows are still deleted (an emptied/secret-only file must not keep
    /// stale vectors), nothing is inserted, and the returned [`ReindexStats`] is all
    /// zero. This is NOT an error.
    /// Single-file convenience wrapper over [`Self::reindex_files`] — same
    /// read-cache → delete → embed → insert contract, batch size 1. Production
    /// full-index runs batch many files per call; this remains the primitive
    /// for tests and the future single-file watcher path.
    #[allow(dead_code)]
    pub(crate) async fn reindex_file(
        &mut self,
        rel_path: &str,
        chunks: &[crate::chunker::Chunk],
        embedder: &dyn crate::embedder::Embedder,
        last_modified: i64,
    ) -> Result<ReindexStats> {
        self.reindex_files(
            &[FileToReindex {
                rel_path,
                chunks,
                last_modified,
            }],
            embedder,
        )
        .await
    }

    /// Delete every chunk row belonging to ANY of `rel_paths`, in ONE Lance
    /// write transaction (`rel_path IN (...)`). Matching per path is exact,
    /// same escaping contract as [`Self::delete_by_file`].
    ///
    /// An empty `rel_paths` is a no-op returning `0` — no predicate, no
    /// version churn.
    pub(crate) async fn delete_by_files(&mut self, rel_paths: &[&str]) -> Result<usize> {
        if rel_paths.is_empty() {
            return Ok(0);
        }

        let table = self.chunks_table().await?;
        let result = table
            .delete(&rel_paths_in_predicate(rel_paths))
            .await
            .map_err(|e| VektorError::Storage(e.to_string()))?;

        let deleted = result.num_deleted_rows as usize;
        if deleted > 0 {
            self.meta.chunks_deleted_since = self
                .meta
                .chunks_deleted_since
                .saturating_add(result.num_deleted_rows);
            self.meta.save(&self.meta_path)?;
        }

        Ok(deleted)
    }

    /// Re-index a BATCH of files in a bounded number of Lance transactions.
    ///
    /// This is the multi-file counterpart of [`Self::reindex_file`] and the
    /// hot path for full-index runs. Per-file reindexing commits two Lance
    /// versions per file (delete + insert), which is O(n²)-shaped as the
    /// version chain and fragment count grow with every file; a 10K-file run
    /// would commit ~20K versions. This method keeps the same
    /// read-cache → delete → embed → insert contract but performs each step
    /// ONCE for the whole batch:
    ///
    /// 1. **Read cache** ([`Self::existing_embeddings_for_files`]) — one query
    ///    for all files' vectors, flattened by `content_hash` (content reuse
    ///    is file-agnostic, so cross-file duplicates embed zero times).
    /// 2. **Delete** ([`Self::delete_by_files`]) — one `rel_path IN (...)`
    ///    transaction for the whole batch. Files with empty `chunks` still
    ///    participate (their stale rows must go).
    /// 3. **Embed** — cache-miss texts across ALL files, deduplicated by
    ///    `content_hash`, in a single `embed_documents` call (the embedder
    ///    batches internally).
    /// 4. **Insert** — rows for all files via [`Self::insert_chunks`] in
    ///    batches of at most [`INSERT_BATCH`] rows.
    ///
    /// Returned [`ReindexStats`] preserve the per-chunk invariant
    /// `reused + embedded == chunks`, where `reused` counts chunks whose
    /// vector came from the store and `embedded` counts chunks that were not
    /// in the store (even when deduplication meant fewer embedder texts).
    pub(crate) async fn reindex_files(
        &mut self,
        files: &[FileToReindex<'_>],
        embedder: &dyn crate::embedder::Embedder,
    ) -> Result<ReindexStats> {
        if files.is_empty() {
            return Ok(ReindexStats::default());
        }

        let rel_paths: Vec<&str> = files.iter().map(|f| f.rel_path).collect();

        // 1. Read existing vectors BEFORE deleting (read-before-delete invariant).
        let mut cache = self.existing_embeddings_for_files(&rel_paths).await?;

        // 2. One delete transaction for the whole batch.
        self.delete_by_files(&rel_paths).await?;

        // 3. Collect cache misses across all files, deduplicated by content
        //    hash, and embed them in one call. Counts are taken against the
        //    ORIGINAL cache so `reused` means "vector came from the store".
        let mut reused = 0usize;
        let mut embedded = 0usize;
        let mut queued: HashSet<&str> = HashSet::new();
        let mut miss_hashes: Vec<String> = Vec::new();
        let mut miss_texts: Vec<String> = Vec::new();
        for file in files {
            for chunk in file.chunks {
                if cache.contains_key(&chunk.content_hash) {
                    reused += 1;
                } else {
                    embedded += 1;
                    if queued.insert(chunk.content_hash.as_str()) {
                        miss_hashes.push(chunk.content_hash.clone());
                        miss_texts.push(chunk.content.clone());
                    }
                }
            }
        }

        if !miss_texts.is_empty() {
            let vectors = embedder.embed_documents(&miss_texts).await?;
            if vectors.len() != miss_texts.len() {
                return Err(VektorError::Storage(format!(
                    "embedder returned {} vectors for {} texts",
                    vectors.len(),
                    miss_texts.len(),
                )));
            }
            for (hash, vector) in miss_hashes.into_iter().zip(vectors) {
                cache.insert(hash, vector);
            }
        }

        // 4. Build rows per file against the now-complete cache (zero embedder
        //    calls inside plan_reindex) and insert in bounded batches.
        let mut rows: Vec<ChunkRow> = Vec::new();
        for file in files {
            let (file_rows, _plan) =
                plan_reindex(file.chunks, &cache, embedder, file.last_modified).await?;
            rows.extend(file_rows);
        }
        for batch in rows.chunks(INSERT_BATCH) {
            self.insert_chunks(batch).await?;
        }

        Ok(ReindexStats {
            chunks: rows.len(),
            embedded,
            reused,
        })
    }

    /// Compact small fragments and prune superseded dataset versions.
    ///
    /// LanceDB is copy-on-write: every delete/insert commits a new immutable
    /// version and leaves the old one on disk. An index run over many files
    /// accrues hundreds of versions and fragments, which bloats disk (~10x)
    /// and slows every subsequent open/scan. Call this once at the END of an
    /// index run — never per file.
    ///
    /// `delete_unverified: true` prunes versions younger than Lance's 7-day
    /// safety window too. That is safe here because Vektor's store is
    /// single-process per project (CLI one-shot or one MCP server; concurrent
    /// writers are already unsupported — state.db would contend first).
    pub(crate) async fn optimize(&mut self) -> Result<()> {
        let table = self.chunks_table().await?;

        table
            .optimize(OptimizeAction::Compact {
                options: CompactionOptions::default(),
                remap_options: None,
            })
            .await
            .map_err(|e| VektorError::Storage(format!("lance compaction failed: {e}")))?;

        table
            .optimize(OptimizeAction::Prune {
                older_than: Some(lancedb::table::optimize::Duration::zero()),
                delete_unverified: Some(true),
                error_if_tagged_old_versions: Some(false),
            })
            .await
            .map_err(|e| VektorError::Storage(format!("lance version prune failed: {e}")))?;

        Ok(())
    }

    // ---------------------------------------------------------------------------
    // 3.8 — Semantic vector search
    // ---------------------------------------------------------------------------

    /// Search the `chunks` table for the nearest neighbours to `query_vec`.
    ///
    /// ## Parameters
    /// - `query_vec`: The query embedding. Must have exactly `self.dim` elements;
    ///   a dimension mismatch returns a [`VektorError::Storage`] immediately,
    ///   without touching LanceDB.
    /// - `top_k`: Maximum number of results to return. `top_k == 0` returns an
    ///   empty `Vec` without issuing any query to LanceDB.
    /// - `filter`: An optional SQL-like predicate string (DataFusion syntax) to
    ///   narrow the search at the LanceDB layer — e.g. `"language = 'rust'"` or
    ///   `"rel_path = 'src/main.rs'"`. Applied as a *pre-filter* so only
    ///   matching rows participate in the ANN scan. The caller is responsible
    ///   for predicate validity and any escaping needed for literal values (use
    ///   [`escape_sql_string_literal`] for single-quoted string constants).
    ///
    /// ## Score semantics
    /// [`SearchResult::score`] is the raw L2 distance from LanceDB's `_distance`
    /// column (auto-projected for every vector query). **Lower is more similar**;
    /// 0.0 is a perfect match. Results are ordered nearest-first (ascending
    /// distance). Phase 4 owns normalisation, RRF fusion, and any
    /// score-inversion needed for display.
    ///
    /// ## Dead-code allowance
    /// This method has no non-test caller until Phase 4 hybrid search (4.5).
    /// `#[allow(dead_code)]` suppresses the lint until then.
    #[allow(dead_code)]
    pub(crate) async fn search(
        &self,
        query_vec: &[f32],
        top_k: usize,
        filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        // --- Validate dimension before any LanceDB call ---
        if query_vec.len() != self.meta.embedding_dim {
            return Err(VektorError::Storage(format!(
                "query vector has {} dimensions but store expects {}; \
                 re-embed with the correct model or re-open with the right dim",
                query_vec.len(),
                self.meta.embedding_dim,
            )));
        }

        // --- Short-circuit for top_k == 0 ---
        if top_k == 0 {
            return Ok(vec![]);
        }

        let table = self.chunks_table().await?;

        // Build the vector query: nearest_to returns VectorQuery (implements QueryBase).
        // `_distance` is auto-projected for all vector queries (disable_scoring_autoprojection
        // defaults to false), so we do not need to select it explicitly.
        let mut vq = table
            .query()
            .nearest_to(query_vec)
            .map_err(|e| VektorError::Storage(format!("nearest_to failed: {e}")))?
            .limit(top_k);

        // Apply the optional caller-supplied filter at the LanceDB query layer.
        if let Some(pred) = filter {
            vq = vq.only_if(pred);
        }

        let stream = vq
            .select(search_result_projection())
            .execute()
            .await
            .map_err(|e| VektorError::Storage(format!("vector search execute failed: {e}")))?;

        let batches: Vec<RecordBatch> = stream.try_collect().await.map_err(|e| {
            VektorError::Storage(format!("vector search stream collect failed: {e}"))
        })?;

        let mut results = search_results_from_batches(&batches, DistanceColumn::Required)?;

        // LanceDB returns rows nearest-first from ANN/flat search.
        // Sort by score ascending (nearest first) to be explicit and robust
        // even when results span multiple batches.
        results.sort_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(results)
    }

    /// Hydrate stored chunk rows by chunk id without materializing vectors.
    ///
    /// Phase 4 hybrid search uses this to fill `HybridResult.content` for
    /// keyword-only hits. Tantivy indexes `content` for BM25 but intentionally
    /// does not store it, so LanceDB remains the full-content source of truth.
    pub(crate) async fn chunks_by_ids(
        &self,
        chunk_ids: &[String],
    ) -> Result<HashMap<String, SearchResult>> {
        if chunk_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let predicate = chunk_ids
            .iter()
            .map(|id| format!("id = '{}'", escape_sql_string_literal(id)))
            .collect::<Vec<_>>()
            .join(" OR ");

        let table = self.chunks_table().await?;
        let stream = table
            .query()
            .only_if(predicate)
            .select(search_result_projection())
            .execute()
            .await
            .map_err(|e| VektorError::Storage(format!("chunk hydration query failed: {e}")))?;
        let batches: Vec<RecordBatch> = stream.try_collect().await.map_err(|e| {
            VektorError::Storage(format!("chunk hydration stream collect failed: {e}"))
        })?;
        let results = search_results_from_batches(&batches, DistanceColumn::Absent)?;

        Ok(results
            .into_iter()
            .map(|result| (result.id.clone(), result))
            .collect())
    }
}

enum DistanceColumn {
    Required,
    Absent,
}

fn search_results_from_batches(
    batches: &[RecordBatch],
    distance_column: DistanceColumn,
) -> Result<Vec<SearchResult>> {
    let mut results = Vec::new();

    for batch in batches {
        let num_rows = batch.num_rows();

        // Helper macro: extract a named column, downcast to the expected type.
        // Returns a VektorError::Storage on missing column or type mismatch.
        macro_rules! col_str {
            ($name:expr) => {{
                batch
                    .column_by_name($name)
                    .ok_or_else(|| {
                        VektorError::Storage(format!("search result missing column '{}'", $name))
                    })?
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| {
                        VektorError::Storage(format!(
                            "column '{}' is not a Utf8 StringArray",
                            $name
                        ))
                    })?
            }};
        }
        macro_rules! col_u32 {
            ($name:expr) => {{
                batch
                    .column_by_name($name)
                    .ok_or_else(|| {
                        VektorError::Storage(format!("search result missing column '{}'", $name))
                    })?
                    .as_any()
                    .downcast_ref::<UInt32Array>()
                    .ok_or_else(|| {
                        VektorError::Storage(format!("column '{}' is not a UInt32Array", $name))
                    })?
            }};
        }
        macro_rules! col_i64 {
            ($name:expr) => {{
                batch
                    .column_by_name($name)
                    .ok_or_else(|| {
                        VektorError::Storage(format!("search result missing column '{}'", $name))
                    })?
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .ok_or_else(|| {
                        VektorError::Storage(format!("column '{}' is not an Int64Array", $name))
                    })?
            }};
        }

        let distances = match distance_column {
            DistanceColumn::Required => Some(
                batch
                    .column_by_name("_distance")
                    .ok_or_else(|| {
                        VektorError::Storage("search result missing column '_distance'".into())
                    })?
                    .as_any()
                    .downcast_ref::<Float32Array>()
                    .ok_or_else(|| {
                        VektorError::Storage("column '_distance' is not a Float32Array".into())
                    })?,
            ),
            DistanceColumn::Absent => None,
        };
        let ids = col_str!("id");
        let content_hashes = col_str!("content_hash");
        let rel_paths = col_str!("rel_path");
        let start_lines = col_u32!("start_line");
        let end_lines = col_u32!("end_line");
        let symbol_names = col_str!("symbol_name");
        let symbol_types = col_str!("symbol_type");
        let languages = col_str!("language");
        let contents = col_str!("content");
        let last_modifieds = col_i64!("last_modified");

        for row in 0..num_rows {
            results.push(SearchResult {
                score: distances
                    .map(|distances| distances.value(row))
                    .unwrap_or(0.0),
                id: ids.value(row).to_string(),
                content_hash: content_hashes.value(row).to_string(),
                rel_path: rel_paths.value(row).to_string(),
                start_line: start_lines.value(row),
                end_line: end_lines.value(row),
                symbol_name: if symbol_names.is_null(row) {
                    None
                } else {
                    Some(symbol_names.value(row).to_string())
                },
                symbol_type: if symbol_types.is_null(row) {
                    None
                } else {
                    Some(symbol_types.value(row).to_string())
                },
                language: languages.value(row).to_string(),
                content: contents.value(row).to_string(),
                last_modified: last_modifieds.value(row),
            });
        }
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// 3.7b — Reindex reuse planner
// ---------------------------------------------------------------------------

/// One file's input to [`VectorStore::reindex_files`]: its exact path, the
/// chunks that should replace its rows, and the mtime stamped on each row.
/// Borrowed views — the CLI's pending-file list owns the data.
pub(crate) struct FileToReindex<'a> {
    pub(crate) rel_path: &'a str,
    pub(crate) chunks: &'a [crate::chunker::Chunk],
    pub(crate) last_modified: i64,
}

/// Build an exact-match `rel_path IN ('a', 'b', ...)` predicate, escaping every
/// path via [`escape_sql_string_literal`] (same contract as the single-file
/// `rel_path = '...'` predicates).
fn rel_paths_in_predicate(rel_paths: &[&str]) -> String {
    let escaped: Vec<String> = rel_paths
        .iter()
        .map(|p| format!("'{}'", escape_sql_string_literal(p)))
        .collect();
    format!("rel_path IN ({})", escaped.join(", "))
}

/// Extract a `content_hash → vector` map from query result batches holding the
/// `content_hash` and `vector` columns (the read half of read-before-delete).
///
/// Columns are read by NAME (not position) — robust and self-documenting.
/// First write wins for duplicate content hashes; identical content ⇒
/// identical embedding, so the choice is immaterial.
fn hash_vector_map_from_batches(batches: &[RecordBatch]) -> Result<HashMap<String, Vec<f32>>> {
    let mut out: HashMap<String, Vec<f32>> = HashMap::new();
    for batch in batches {
        let hashes = batch
            .column_by_name("content_hash")
            .ok_or_else(|| VektorError::Storage("query result missing content_hash column".into()))?
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| {
                VektorError::Storage("content_hash column is not a Utf8 StringArray".into())
            })?;

        let vectors = batch
            .column_by_name("vector")
            .ok_or_else(|| VektorError::Storage("query result missing vector column".into()))?
            .as_any()
            .downcast_ref::<FixedSizeListArray>()
            .ok_or_else(|| {
                VektorError::Storage("vector column is not a FixedSizeListArray".into())
            })?;

        for row in 0..batch.num_rows() {
            if hashes.is_null(row) {
                return Err(VektorError::Storage(
                    "unexpected null content_hash in chunks table".into(),
                ));
            }
            let content_hash = hashes.value(row).to_string();

            // Reconstruct the per-row `Vec<f32>` from the FixedSizeList: each
            // list cell is itself an array; downcast that inner array to a
            // Float32Array and copy its values out.
            let cell = vectors.value(row);
            let floats = cell
                .as_any()
                .downcast_ref::<Float32Array>()
                .ok_or_else(|| VektorError::Storage("vector list items are not Float32".into()))?;
            let vector: Vec<f32> = floats.values().to_vec();

            match out.entry(content_hash) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(vector);
                }
                std::collections::hash_map::Entry::Occupied(e) => {
                    tracing::debug!(
                        content_hash = e.key().as_str(),
                        "duplicate content_hash in read-cache query; keeping first vector"
                    );
                }
            }
        }
    }

    Ok(out)
}

/// Per-file outcome of [`VectorStore::reindex_file`].
///
/// `reused + embedded == chunks` (one row inserted per input chunk). All three
/// are zero for an empty/secret-only file. The CLI and MCP handler aggregate
/// these across files into the run-level [`crate::cli::IndexStats`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReindexStats {
    /// Total chunk rows (re)inserted for this file.
    pub chunks: usize,
    /// Rows whose vector was freshly embedded (cache miss).
    pub embedded: usize,
    /// Rows whose vector was reused from the existing-embeddings cache.
    pub reused: usize,
}

/// Counts of reused vs. freshly embedded chunks from [`plan_reindex`].
///
/// `reused` + `embedded` always equals the number of input chunks.
/// 3.7c reads these for logging / metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReindexPlan {
    /// Chunks whose vector was reused from the existing-embeddings cache.
    pub reused: usize,
    /// Chunks that were freshly embedded (cache miss).
    pub embedded: usize,
}

/// Reuse-diff planner for a single file's re-indexing pass.
///
/// Given the file's current chunks and its existing-embeddings cache (keyed by
/// `content_hash`, from [`VectorStore::existing_embeddings_by_content_hash`]),
/// this function decides which chunks can reuse a cached vector and which must
/// be embedded fresh.  Only the cache-miss chunks are passed to the embedder —
/// in a **single** `embed_documents` call (document-prefix mode) — so
/// unchanged functions never trigger redundant ONNX/cloud inference.
///
/// ## Returns
/// `(records, plan)` where `records` is in the **same order** as `chunks`
/// (deterministic for search-result metadata) and `plan` carries the counts
/// 3.7c uses for logging.
///
/// ## Empty chunk list
/// Returns `(vec![], ReindexPlan { reused: 0, embedded: 0 })` immediately,
/// making **no** embedder call.  The delete of old rows is 3.7c's concern.
///
/// ## Chunk → ChunkRow mapping
/// - `start_line` / `end_line`: `usize` → `u32` via checked cast (errors on
///   truncation — practically impossible, but correct to check).
/// - `language`: `Option<Language>` → non-null `String`; `Some(l)` →
///   `l.as_str()`, `None` → `"unknown"`.  The `language` column is non-null.
/// - `last_modified` is supplied by the caller (3.7c passes the file's mtime).
pub(crate) async fn plan_reindex(
    chunks: &[crate::chunker::Chunk],
    cache: &HashMap<String, Vec<f32>>,
    embedder: &dyn crate::embedder::Embedder,
    last_modified: i64,
) -> crate::error::Result<(Vec<ChunkRow>, ReindexPlan)> {
    if chunks.is_empty() {
        return Ok((
            vec![],
            ReindexPlan {
                reused: 0,
                embedded: 0,
            },
        ));
    }

    // Partition: collect indices + content for cache-miss chunks to embed.
    let mut miss_indices: Vec<usize> = Vec::new();
    let mut miss_texts: Vec<String> = Vec::new();

    for (i, chunk) in chunks.iter().enumerate() {
        if !cache.contains_key(&chunk.content_hash) {
            miss_indices.push(i);
            miss_texts.push(chunk.content.clone());
        }
    }

    // ONE embed_documents call for all cache-miss chunks (document-prefix mode).
    let fresh_vectors: Vec<Vec<f32>> = if miss_texts.is_empty() {
        vec![]
    } else {
        embedder.embed_documents(&miss_texts).await?
    };

    let embedded_count = miss_indices.len();
    let reused_count = chunks.len() - embedded_count;

    // Map freshly-embedded vectors back to their chunk indices.
    // miss_indices[j] → fresh_vectors[j].
    let mut fresh_iter = miss_indices.into_iter().zip(fresh_vectors.into_iter());
    let mut next_fresh: Option<(usize, Vec<f32>)> = fresh_iter.next();

    // Assemble output in original chunk order.
    let mut records: Vec<ChunkRow> = Vec::with_capacity(chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        let vector = if let Some(cached) = cache.get(&chunk.content_hash) {
            // Cache hit: reuse stored vector.
            cached.clone()
        } else {
            // Cache miss: consume the next fresh vector (must be present).
            let (idx, vec) = next_fresh
                .take()
                .expect("fresh_iter must have an entry for every cache-miss index");
            debug_assert_eq!(idx, i, "fresh_iter and chunk index are in sync");
            next_fresh = fresh_iter.next();
            vec
        };

        let start_line = u32::try_from(chunk.start_line).map_err(|_| {
            crate::error::VektorError::Storage(format!(
                "chunk start_line {} overflows u32 (rel_path={})",
                chunk.start_line, chunk.rel_path
            ))
        })?;
        let end_line = u32::try_from(chunk.end_line).map_err(|_| {
            crate::error::VektorError::Storage(format!(
                "chunk end_line {} overflows u32 (rel_path={})",
                chunk.end_line, chunk.rel_path
            ))
        })?;
        let language = chunk
            .language
            .map(|l| l.as_str().to_owned())
            .unwrap_or_else(|| "unknown".to_owned());

        records.push(ChunkRow {
            id: chunk.id.clone(),
            content_hash: chunk.content_hash.clone(),
            vector,
            rel_path: chunk.rel_path.clone(),
            start_line,
            end_line,
            symbol_name: chunk.symbol_name.clone(),
            symbol_type: chunk.symbol_type.clone(),
            language,
            content: chunk.content.clone(),
            last_modified,
        });
    }

    Ok((
        records,
        ReindexPlan {
            reused: reused_count,
            embedded: embedded_count,
        },
    ))
}

// ---------------------------------------------------------------------------
// 3.8 — SearchResult type
// ---------------------------------------------------------------------------

/// A single result row returned by [`VectorStore::search`].
///
/// Every metadata field from the `chunks` table schema is present except the
/// raw `vector` column (Phase 4 hybrid fusion and display never need to
/// re-inspect the stored embedding; they operate on `score` + text metadata).
///
/// ## `score` semantics
/// `score` is the raw L2 (Euclidean squared) distance that LanceDB stores in
/// the auto-projected `_distance` column for an ANN/flat vector search. This
/// means **lower is more similar** — a perfect match has distance 0.0. Results
/// are returned nearest-first (ascending score). Phase 4 owns normalization,
/// RRF fusion, and any score-inversion needed for display.
///
/// `#[allow(dead_code)]`: consumed by Phase 4 hybrid search (4.5);
/// allow until then.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    /// Raw L2 distance from the query vector (lower = more similar).
    pub score: f32,
    pub id: String,
    pub content_hash: String,
    pub rel_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub symbol_name: Option<String>,
    pub symbol_type: Option<String>,
    pub language: String,
    pub content: String,
    pub last_modified: i64,
}

/// Arrow schema for the `chunks` table.
///
/// Column order/names/types are the storage contract for tasks 3.7a/3.7c/3.8/3.9.
/// The `vector` column is a fixed-size list of `Float32` of length `dim`.
fn chunks_schema(dim: usize) -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("content_hash", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dim as i32,
            ),
            false,
        ),
        Field::new("rel_path", DataType::Utf8, false),
        Field::new("start_line", DataType::UInt32, false),
        Field::new("end_line", DataType::UInt32, false),
        Field::new("symbol_name", DataType::Utf8, true),
        Field::new("symbol_type", DataType::Utf8, true),
        Field::new("language", DataType::Utf8, false),
        Field::new("content", DataType::Utf8, false),
        Field::new("last_modified", DataType::Int64, false),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIM: usize = 768;
    const MODEL: &str = "jinaai/jina-embeddings-v2-base-code";

    /// Point `data_dir` at an explicit tempdir so the store never touches the
    /// developer's real `~/.vektor`. Because the path is absolute there is no
    /// `~` expansion, hence no need to isolate `HOME` (which also lets these
    /// tests run on the async runtime without process-global env guards).
    fn config_with_data_dir(data_dir: &Path) -> Config {
        Config {
            index: crate::config::IndexConfig {
                data_dir: data_dir.to_string_lossy().into_owned(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Build a minimal valid `ChunkRow` for `rel_path` (one row per `id`). The
    /// vector is a constant `DIM`-length vector — content is irrelevant to the
    /// delete-by-path contract, only `rel_path` matters here.
    fn row(id: &str, rel_path: &str) -> ChunkRow {
        ChunkRow {
            id: id.to_string(),
            content_hash: format!("hash-{id}"),
            vector: vec![0.1_f32; DIM],
            rel_path: rel_path.to_string(),
            start_line: 1,
            end_line: 10,
            symbol_name: Some("fn_name".to_string()),
            symbol_type: Some("function".to_string()),
            language: "rust".to_string(),
            content: format!("content of {id}"),
            last_modified: 1_700_000_000,
        }
    }

    /// Count rows in the chunks table whose `rel_path` equals `rel_path`.
    async fn count_for_path(store: &VectorStore, rel_path: &str) -> usize {
        let table = store.chunks_table().await.expect("open table");
        let predicate = format!("rel_path = '{}'", escape_sql_string_literal(rel_path));
        table.count_rows(Some(predicate)).await.expect("count rows")
    }

    /// Total rows in the chunks table.
    async fn count_all(store: &VectorStore) -> usize {
        let table = store.chunks_table().await.expect("open table");
        table.count_rows(None).await.expect("count rows")
    }

    /// Build a `ChunkRow` with an explicit `content_hash` and a distinct vector
    /// so cache-read tests can assert exactly which hash maps to which vector.
    /// The vector is `[seed, seed+1, ...]` (DIM-length) — distinct per `seed` and
    /// not all-equal, so a "values preserved" assertion is meaningful.
    fn row_with(id: &str, rel_path: &str, content_hash: &str, seed: f32) -> ChunkRow {
        let vector: Vec<f32> = (0..DIM).map(|i| seed + i as f32).collect();
        ChunkRow {
            id: id.to_string(),
            content_hash: content_hash.to_string(),
            vector,
            rel_path: rel_path.to_string(),
            start_line: 1,
            end_line: 10,
            symbol_name: Some("fn_name".to_string()),
            symbol_type: Some("function".to_string()),
            language: "rust".to_string(),
            content: format!("content of {id}"),
            last_modified: 1_700_000_000,
        }
    }

    #[test]
    fn escape_sql_string_literal_doubles_single_quotes() {
        // A bare path is untouched.
        assert_eq!(escape_sql_string_literal("src/main.rs"), "src/main.rs");
        // A single quote is doubled (SQL literal escaping).
        assert_eq!(escape_sql_string_literal("a'b.rs"), "a''b.rs");
        // Multiple quotes each doubled.
        assert_eq!(escape_sql_string_literal("''"), "''''");
        // An injection attempt becomes an inert literal, not breaking the quote.
        assert_eq!(
            escape_sql_string_literal("x' OR '1'='1"),
            "x'' OR ''1''=''1"
        );
        // Double quotes, spaces, and backslashes are literal inside single
        // quotes — they are NOT escaped (SQL literals are not C-style).
        assert_eq!(escape_sql_string_literal(r#"a b"c\d.rs"#), r#"a b"c\d.rs"#);
    }

    #[tokio::test]
    async fn delete_by_file_removes_only_target_rows() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        store
            .insert_chunks(&[
                row("a1", "src/a.rs"),
                row("a2", "src/a.rs"),
                row("b1", "src/b.rs"),
            ])
            .await
            .expect("seed rows");

        assert_eq!(count_all(&store).await, 3);

        let deleted = store.delete_by_file("src/a.rs").await.expect("delete");

        assert_eq!(deleted, 2, "both rows for the target file are deleted");
        assert_eq!(
            count_for_path(&store, "src/a.rs").await,
            0,
            "target file rows are gone"
        );
        assert_eq!(
            count_for_path(&store, "src/b.rs").await,
            1,
            "other file's row survives"
        );
        assert_eq!(count_all(&store).await, 1);
    }

    #[tokio::test]
    async fn insert_chunks_increments_and_persists_inserted_churn() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");
        assert_eq!(store.meta().chunks_inserted_since, 0);

        store
            .insert_chunks(&[row("a1", "src/a.rs"), row("a2", "src/a.rs")])
            .await
            .expect("insert rows");

        assert_eq!(store.meta().chunks_inserted_since, 2);
        drop(store);

        let reopened = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("reopen store");
        assert_eq!(reopened.meta().chunks_inserted_since, 2);
    }

    #[tokio::test]
    async fn delete_by_file_is_exact_match_not_prefix() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        store
            .insert_chunks(&[
                row("t", "src/a.rs"),
                row("bak", "src/a.rs.bak"),
                row("nested", "src/a/b.rs"),
            ])
            .await
            .expect("seed rows");

        let deleted = store.delete_by_file("src/a.rs").await.expect("delete");

        assert_eq!(deleted, 1, "only the exact path matches");
        assert_eq!(count_for_path(&store, "src/a.rs.bak").await, 1);
        assert_eq!(count_for_path(&store, "src/a/b.rs").await, 1);
    }

    #[tokio::test]
    async fn delete_by_file_missing_path_is_ok_and_returns_zero() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        store
            .insert_chunks(&[row("b1", "src/b.rs")])
            .await
            .expect("seed rows");

        let before = store.meta().chunks_deleted_since;
        let deleted = store
            .delete_by_file("does/not/exist.rs")
            .await
            .expect("missing path is not an error");

        assert_eq!(deleted, 0);
        assert_eq!(count_all(&store).await, 1, "untouched");
        assert_eq!(
            store.meta().chunks_deleted_since,
            before,
            "no churn recorded for a zero-row delete"
        );
    }

    #[tokio::test]
    async fn delete_by_file_handles_quoted_and_special_paths() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        // A path containing a single quote (the dangerous char), plus a benign
        // neighbor that must survive — proves escaping targets the right rows.
        let quoted = "src/o'brien's file.rs";
        store
            .insert_chunks(&[
                row("q1", quoted),
                row("q2", quoted),
                row("safe", r#"src/a b"c\d.rs"#),
            ])
            .await
            .expect("seed rows");

        let deleted = store.delete_by_file(quoted).await.expect("delete quoted");

        assert_eq!(deleted, 2, "both quoted-path rows deleted");
        assert_eq!(count_for_path(&store, quoted).await, 0);
        assert_eq!(
            count_for_path(&store, r#"src/a b"c\d.rs"#).await,
            1,
            "special-char neighbor survives"
        );

        // Delete the special-char (double-quote/space/backslash) path too.
        let deleted2 = store
            .delete_by_file(r#"src/a b"c\d.rs"#)
            .await
            .expect("delete special");
        assert_eq!(deleted2, 1);
        assert_eq!(count_all(&store).await, 0);
    }

    #[tokio::test]
    async fn delete_by_file_increments_and_persists_deleted_churn() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        store
            .insert_chunks(&[row("a1", "src/a.rs"), row("a2", "src/a.rs")])
            .await
            .expect("seed rows");

        assert_eq!(store.meta().chunks_deleted_since, 0);
        store.delete_by_file("src/a.rs").await.expect("delete");
        assert_eq!(store.meta().chunks_deleted_since, 2, "in-memory churn");

        // Persisted to the sidecar: reopening the store reads the updated stat
        // back (which also exercises the atomic save path).
        drop(store);
        let reopened = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("reopen store");
        assert_eq!(
            reopened.meta().chunks_deleted_since,
            2,
            "churn persisted across reopen"
        );
    }

    #[tokio::test]
    async fn existing_embeddings_by_content_hash_filters_by_rel_path() {
        // Two files SHARE a content_hash but have DIFFERENT vectors. Querying one
        // file must return only that file's vector for the shared hash — proving
        // the rel_path filter is enforced (no cross-file leakage).
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        let shared = "shared-hash";
        store
            .insert_chunks(&[
                // file A: the shared hash plus a unique one.
                row_with("a1", "src/a.rs", shared, 10.0),
                row_with("a2", "src/a.rs", "only-in-a", 20.0),
                // file B: same shared hash, but a DIFFERENT vector (seed 99).
                row_with("b1", "src/b.rs", shared, 99.0),
            ])
            .await
            .expect("seed rows");

        let map = store
            .existing_embeddings_by_content_hash("src/a.rs")
            .await
            .expect("read cache");

        assert_eq!(map.len(), 2, "only file A's two hashes are returned");
        assert!(map.contains_key("only-in-a"));

        // The shared hash must resolve to file A's vector (seed 10), NOT B's (99).
        let expected_a: Vec<f32> = (0..DIM).map(|i| 10.0 + i as f32).collect();
        assert_eq!(
            map.get(shared),
            Some(&expected_a),
            "shared hash resolves to THIS file's vector, not the other file's"
        );
    }

    #[tokio::test]
    async fn existing_embeddings_by_content_hash_preserves_dimension_and_values() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        store
            .insert_chunks(&[row_with("a1", "src/a.rs", "h-a1", 3.5)])
            .await
            .expect("seed rows");

        let map = store
            .existing_embeddings_by_content_hash("src/a.rs")
            .await
            .expect("read cache");

        let v = map.get("h-a1").expect("hash present");
        assert_eq!(v.len(), DIM, "full dimension preserved");
        let expected: Vec<f32> = (0..DIM).map(|i| 3.5 + i as f32).collect();
        assert_eq!(v, &expected, "all values preserved exactly");
    }

    #[tokio::test]
    async fn existing_embeddings_by_content_hash_missing_file_is_empty_map() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        // Seed an unrelated file so the table is non-empty.
        store
            .insert_chunks(&[row_with("b1", "src/b.rs", "h-b1", 1.0)])
            .await
            .expect("seed rows");

        let map = store
            .existing_embeddings_by_content_hash("does/not/exist.rs")
            .await
            .expect("missing file is not an error");

        assert!(map.is_empty(), "no rows for a missing file => empty map");
    }

    #[tokio::test]
    async fn existing_embeddings_by_content_hash_dedupes_within_one_file() {
        // The same content_hash appears twice within ONE file (identical content
        // co-located). Exactly one vector is kept (first wins).
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        let dup = "dup-hash";
        store
            .insert_chunks(&[
                row_with("a1", "src/a.rs", dup, 1.0),
                row_with("a2", "src/a.rs", dup, 2.0),
                row_with("a3", "src/a.rs", "unique", 7.0),
            ])
            .await
            .expect("seed rows");

        let map = store
            .existing_embeddings_by_content_hash("src/a.rs")
            .await
            .expect("read cache");

        assert_eq!(map.len(), 2, "duplicate hash collapses to one entry");
        let dup_vec = map.get(dup).expect("dup hash present");
        assert_eq!(dup_vec.len(), DIM, "kept vector has full dimension");
        // Either seeded vector is acceptable per the contract (identical content
        // ⇒ identical embedding); assert it is one of the two we wrote.
        let opt1: Vec<f32> = (0..DIM).map(|i| 1.0 + i as f32).collect();
        let opt2: Vec<f32> = (0..DIM).map(|i| 2.0 + i as f32).collect();
        assert!(
            *dup_vec == opt1 || *dup_vec == opt2,
            "kept vector is one of the seeded duplicates"
        );
    }

    #[tokio::test]
    async fn existing_embeddings_by_content_hash_is_side_effect_free() {
        // Reading the cache must NOT delete or mutate rows: the data is still
        // present immediately afterward (read-before-delete invariant), and the
        // deletion churn stat is untouched.
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        store
            .insert_chunks(&[
                row_with("a1", "src/a.rs", "h-a1", 1.0),
                row_with("a2", "src/a.rs", "h-a2", 2.0),
            ])
            .await
            .expect("seed rows");

        let before_churn = store.meta().chunks_deleted_since;

        let map = store
            .existing_embeddings_by_content_hash("src/a.rs")
            .await
            .expect("read cache");
        assert_eq!(map.len(), 2);

        // Rows are still there after the read (the read-before-delete contract).
        assert_eq!(
            count_for_path(&store, "src/a.rs").await,
            2,
            "read must not delete rows"
        );
        assert_eq!(count_all(&store).await, 2, "no rows mutated/removed");
        assert_eq!(
            store.meta().chunks_deleted_since,
            before_churn,
            "read records no deletion churn"
        );
    }

    #[tokio::test]
    async fn new_creates_project_scoped_lance_dir_and_metadata() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");
        let lance_dir = store.lance_dir().to_path_buf();
        let meta = store.meta().clone();

        assert!(lance_dir.starts_with(data_dir.path()));
        assert!(lance_dir.ends_with("lance"));
        assert!(lance_dir.exists(), "lance dir should be created");
        // The project-hash directory sits between data_dir and lance/, so
        // lance/'s parent is NOT data_dir itself.
        assert_ne!(lance_dir.parent(), Some(data_dir.path()));

        assert_eq!(meta.embedding_dim, DIM);
        assert_eq!(meta.model_name, MODEL);
        assert_eq!(meta.vektor_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(meta.last_full_index_at, None);
        assert_eq!(meta.chunks_at_last_ann_rebuild, 0);
    }

    #[tokio::test]
    async fn chunks_table_schema_matches_contract() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");
        let table = store.chunks_table().await.expect("open chunks table");
        let schema = table.schema().await.expect("table schema");

        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(
            names,
            vec![
                "id",
                "content_hash",
                "vector",
                "rel_path",
                "start_line",
                "end_line",
                "symbol_name",
                "symbol_type",
                "language",
                "content",
                "last_modified",
            ]
        );

        let id = schema.field_with_name("id").expect("id field");
        assert_eq!(id.data_type(), &DataType::Utf8);

        let start = schema.field_with_name("start_line").expect("start_line");
        assert_eq!(start.data_type(), &DataType::UInt32);

        let last_modified = schema
            .field_with_name("last_modified")
            .expect("last_modified");
        assert_eq!(last_modified.data_type(), &DataType::Int64);

        // Vector column must be FixedSizeList<Float32, DIM>.
        let vector = schema.field_with_name("vector").expect("vector field");
        match vector.data_type() {
            DataType::FixedSizeList(item, size) => {
                assert_eq!(*size, DIM as i32);
                assert_eq!(item.data_type(), &DataType::Float32);
            }
            other => panic!("vector column must be FixedSizeList, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reopen_same_dim_and_model_succeeds() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let first = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");
        let first_dir = first.lance_dir().to_path_buf();
        drop(first);

        let second = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("reopen store");

        assert_eq!(second.lance_dir(), first_dir.as_path());
        assert_eq!(second.meta().embedding_dim, DIM);
        // Reopen must not duplicate the table.
        let names = second
            .conn
            .table_names()
            .execute()
            .await
            .expect("table names");
        assert_eq!(names.iter().filter(|n| n.as_str() == "chunks").count(), 1);
    }

    #[tokio::test]
    async fn reopen_with_different_dim_returns_reindex_error() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        let err = match VectorStore::new(project.path(), &config, 384, MODEL).await {
            Ok(_) => panic!("dim mismatch must error"),
            Err(e) => e,
        };

        assert!(matches!(err, VektorError::Storage(_)));
        let msg = err.to_string();
        assert!(msg.contains("dimension changed"), "got: {msg}");
        assert!(msg.contains("re-index required"), "got: {msg}");
    }

    #[tokio::test]
    async fn reopen_with_different_model_returns_reindex_error() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        let err = match VectorStore::new(project.path(), &config, DIM, "some/other-model").await {
            Ok(_) => panic!("model mismatch must error"),
            Err(e) => e,
        };

        assert!(matches!(err, VektorError::Storage(_)));
        assert!(err.to_string().contains("model changed"));
    }

    #[tokio::test]
    async fn zero_dim_is_rejected() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let err = match VectorStore::new(project.path(), &config, 0, MODEL).await {
            Ok(_) => panic!("zero dim must error"),
            Err(e) => e,
        };
        assert!(matches!(err, VektorError::Storage(_)));
    }

    // -----------------------------------------------------------------------
    // 3.7b — plan_reindex tests
    // -----------------------------------------------------------------------

    use crate::chunker::{Chunk, Language};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Fake embedder that counts how many texts it was asked to embed and
    /// returns deterministic vectors: each element is `(text.len() as f32)`.
    /// `dim` must be >= 1; the vector is filled with the same scalar.
    struct CountingEmbedder {
        dim: usize,
        call_count: AtomicUsize,
        text_count: AtomicUsize,
    }

    impl CountingEmbedder {
        fn new(dim: usize) -> Self {
            Self {
                dim,
                call_count: AtomicUsize::new(0),
                text_count: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }

        fn texts_embedded(&self) -> usize {
            self.text_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl crate::embedder::Embedder for CountingEmbedder {
        async fn embed(&self, texts: &[String]) -> crate::error::Result<Vec<Vec<f32>>> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            self.text_count.fetch_add(texts.len(), Ordering::SeqCst);
            // Deterministic vector: each f32 = text.len() as f32.
            Ok(texts
                .iter()
                .map(|t| vec![t.len() as f32; self.dim])
                .collect())
        }

        fn dim(&self) -> usize {
            self.dim
        }

        fn name(&self) -> &str {
            "counting-embedder"
        }

        fn prefix_for_document(&self) -> &str {
            ""
        }

        fn prefix_for_query(&self) -> &str {
            ""
        }
    }

    /// Build a minimal `Chunk` for testing the planner.  `language=None` by
    /// default; pass `Some(Language::Rust)` where language mapping is tested.
    fn make_chunk(
        id: &str,
        content: &str,
        content_hash: &str,
        rel_path: &str,
        start_line: usize,
        end_line: usize,
        language: Option<Language>,
    ) -> Chunk {
        Chunk {
            id: id.to_string(),
            content: content.to_string(),
            content_hash: content_hash.to_string(),
            rel_path: rel_path.to_string(),
            start_line,
            end_line,
            symbol_name: None,
            symbol_type: None,
            language,
        }
    }

    // --- AC: Empty chunk list → no embedder call, empty output ---

    #[tokio::test]
    async fn reindex_plan_empty_chunks_makes_no_embedder_call() {
        let embedder = CountingEmbedder::new(4);
        let cache: HashMap<String, Vec<f32>> = HashMap::new();

        let (records, plan) = plan_reindex(&[], &cache, &embedder, 0)
            .await
            .expect("plan_reindex must succeed");

        assert_eq!(records.len(), 0, "no output rows for empty input");
        assert_eq!(embedder.calls(), 0, "embedder must not be called");
        assert_eq!(embedder.texts_embedded(), 0);
        assert_eq!(plan.reused, 0);
        assert_eq!(plan.embedded, 0);
    }

    // --- AC: All chunks present in cache → 0 embedder calls, all vectors reused ---

    #[tokio::test]
    async fn reindex_plan_all_cached_reuses_all_vectors() {
        let embedder = CountingEmbedder::new(4);
        let chunks = vec![
            make_chunk("c1", "fn foo() {}", "hash-foo", "src/a.rs", 1, 5, None),
            make_chunk("c2", "fn bar() {}", "hash-bar", "src/a.rs", 6, 10, None),
        ];
        let cached_foo: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0];
        let cached_bar: Vec<f32> = vec![5.0, 6.0, 7.0, 8.0];
        let mut cache: HashMap<String, Vec<f32>> = HashMap::new();
        cache.insert("hash-foo".to_string(), cached_foo.clone());
        cache.insert("hash-bar".to_string(), cached_bar.clone());

        let (records, plan) = plan_reindex(&chunks, &cache, &embedder, 1_700_000_000)
            .await
            .expect("plan_reindex must succeed");

        assert_eq!(
            embedder.calls(),
            0,
            "no embedder call when all chunks cached"
        );
        assert_eq!(embedder.texts_embedded(), 0);
        assert_eq!(records.len(), 2);
        assert_eq!(plan.reused, 2);
        assert_eq!(plan.embedded, 0);

        // Vectors are the cached ones, not freshly generated.
        assert_eq!(records[0].vector, cached_foo, "chunk 0 uses cached vector");
        assert_eq!(records[1].vector, cached_bar, "chunk 1 uses cached vector");
    }

    // --- AC: One chunk's content_hash NOT in cache → embedder called with exactly 1 text ---

    #[tokio::test]
    async fn reindex_plan_one_miss_embeds_only_changed_chunk() {
        let embedder = CountingEmbedder::new(4);
        let chunks = vec![
            make_chunk("c1", "fn foo() {}", "hash-foo", "src/a.rs", 1, 5, None),
            make_chunk(
                "c2",
                "fn bar_new() {}",
                "hash-bar-new",
                "src/a.rs",
                6,
                10,
                None,
            ),
            make_chunk("c3", "fn baz() {}", "hash-baz", "src/a.rs", 11, 15, None),
        ];
        let cached_foo: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0];
        let cached_baz: Vec<f32> = vec![9.0, 9.0, 9.0, 9.0];
        let mut cache: HashMap<String, Vec<f32>> = HashMap::new();
        cache.insert("hash-foo".to_string(), cached_foo.clone());
        // hash-bar-new NOT in cache (simulates one-function edit)
        cache.insert("hash-baz".to_string(), cached_baz.clone());

        let (records, plan) = plan_reindex(&chunks, &cache, &embedder, 1_700_000_000)
            .await
            .expect("plan_reindex must succeed");

        // Embedder called once with exactly 1 text — the changed chunk's content.
        assert_eq!(
            embedder.calls(),
            1,
            "exactly one embed_documents call for one miss"
        );
        assert_eq!(
            embedder.texts_embedded(),
            1,
            "exactly 1 text passed to embedder"
        );
        assert_eq!(plan.reused, 2);
        assert_eq!(plan.embedded, 1);
        assert_eq!(records.len(), 3);

        // Cached chunks keep their vectors.
        assert_eq!(records[0].vector, cached_foo, "chunk 0 (foo) reuses cache");
        assert_eq!(records[2].vector, cached_baz, "chunk 2 (baz) reuses cache");

        // The freshly embedded chunk gets the CountingEmbedder's deterministic vector.
        // The content "fn bar_new() {}" has len=16, so each f32 = 16.0 (no prefix).
        let expected_fresh = vec!["fn bar_new() {}".len() as f32; 4];
        assert_eq!(
            records[1].vector, expected_fresh,
            "chunk 1 (bar_new) gets fresh embedding"
        );
    }

    // --- AC: New file (empty cache) → all chunks embedded ---

    #[tokio::test]
    async fn reindex_plan_empty_cache_embeds_all_chunks() {
        let embedder = CountingEmbedder::new(4);
        let chunks = vec![
            make_chunk("c1", "fn a() {}", "hash-a", "src/new.rs", 1, 3, None),
            make_chunk("c2", "fn b() {}", "hash-b", "src/new.rs", 4, 6, None),
        ];
        let cache: HashMap<String, Vec<f32>> = HashMap::new(); // empty — new file

        let (records, plan) = plan_reindex(&chunks, &cache, &embedder, 1_700_000_001)
            .await
            .expect("plan_reindex must succeed");

        assert_eq!(
            embedder.calls(),
            1,
            "one embed_documents call for all chunks"
        );
        assert_eq!(embedder.texts_embedded(), 2, "both chunks embedded");
        assert_eq!(plan.reused, 0);
        assert_eq!(plan.embedded, 2);
        assert_eq!(records.len(), 2);

        // CountingEmbedder returns vec![content.len() as f32; dim] (no prefix).
        let expected_a = vec!["fn a() {}".len() as f32; 4];
        let expected_b = vec!["fn b() {}".len() as f32; 4];
        assert_eq!(records[0].vector, expected_a);
        assert_eq!(records[1].vector, expected_b);
    }

    // --- AC: Output ChunkRow order matches input chunk order ---

    #[tokio::test]
    async fn reindex_plan_output_order_matches_input_order() {
        let embedder = CountingEmbedder::new(4);
        // Alternating cache hits and misses to stress ordering.
        let chunks = vec![
            make_chunk("c1", "content-A", "hash-A", "src/x.rs", 1, 2, None), // miss
            make_chunk("c2", "content-B", "hash-B", "src/x.rs", 3, 4, None), // hit
            make_chunk("c3", "content-C", "hash-C", "src/x.rs", 5, 6, None), // miss
            make_chunk("c4", "content-D", "hash-D", "src/x.rs", 7, 8, None), // hit
        ];
        let cached_b: Vec<f32> = vec![2.0, 2.0, 2.0, 2.0];
        let cached_d: Vec<f32> = vec![4.0, 4.0, 4.0, 4.0];
        let mut cache: HashMap<String, Vec<f32>> = HashMap::new();
        cache.insert("hash-B".to_string(), cached_b.clone());
        cache.insert("hash-D".to_string(), cached_d.clone());

        let (records, plan) = plan_reindex(&chunks, &cache, &embedder, 0)
            .await
            .expect("plan_reindex must succeed");

        assert_eq!(records.len(), 4);
        assert_eq!(plan.reused, 2);
        assert_eq!(plan.embedded, 2);

        // Verify IDs are in original order.
        let ids: Vec<&str> = records.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["c1", "c2", "c3", "c4"]);

        // Cache hits keep their vectors.
        assert_eq!(records[1].vector, cached_b);
        assert_eq!(records[3].vector, cached_d);

        // Cache misses got fresh embeddings (CountingEmbedder: vec![content.len(); 4]).
        let expected_a = vec!["content-A".len() as f32; 4];
        let expected_c = vec!["content-C".len() as f32; 4];
        assert_eq!(records[0].vector, expected_a);
        assert_eq!(records[2].vector, expected_c);
    }

    // --- AC: language None → "unknown"; language Some → as_str(); line numbers as u32 ---

    #[tokio::test]
    async fn reindex_plan_maps_language_and_line_numbers_correctly() {
        let embedder = CountingEmbedder::new(4);
        let chunks = vec![
            make_chunk(
                "rust-fn",
                "fn r() {}",
                "h-rust",
                "src/r.rs",
                10,
                20,
                Some(Language::Rust),
            ),
            make_chunk("unknown-fn", "def p():", "h-py", "src/u.rs", 1, 5, None),
            make_chunk(
                "ts-fn",
                "function t() {}",
                "h-ts",
                "src/t.ts",
                100,
                200,
                Some(Language::TypeScript),
            ),
        ];
        let cache: HashMap<String, Vec<f32>> = HashMap::new();

        let (records, _plan) = plan_reindex(&chunks, &cache, &embedder, 9999)
            .await
            .expect("plan_reindex must succeed");

        assert_eq!(records.len(), 3);

        // Language mapping.
        assert_eq!(records[0].language, "rust");
        assert_eq!(records[1].language, "unknown");
        assert_eq!(records[2].language, "typescript");

        // Line number u32 mapping.
        assert_eq!(records[0].start_line, 10u32);
        assert_eq!(records[0].end_line, 20u32);
        assert_eq!(records[2].start_line, 100u32);
        assert_eq!(records[2].end_line, 200u32);

        // last_modified propagated.
        for r in &records {
            assert_eq!(r.last_modified, 9999);
        }
    }

    // -----------------------------------------------------------------------
    // 3.7c — reindex_file tests
    // -----------------------------------------------------------------------

    /// Read a chunk's stored vector by `content_hash` from the live table.
    async fn vector_for(
        store: &VectorStore,
        rel_path: &str,
        content_hash: &str,
    ) -> Option<Vec<f32>> {
        store
            .existing_embeddings_by_content_hash(rel_path)
            .await
            .expect("read cache")
            .get(content_hash)
            .cloned()
    }

    #[tokio::test]
    async fn reindex_file_first_pass_embeds_all_and_inserts_rows() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());
        let embedder = CountingEmbedder::new(DIM);

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        let chunks = vec![
            make_chunk(
                "c1",
                "fn a() {}",
                "h-a",
                "src/a.rs",
                1,
                3,
                Some(Language::Rust),
            ),
            make_chunk(
                "c2",
                "fn b() {}",
                "h-b",
                "src/a.rs",
                4,
                6,
                Some(Language::Rust),
            ),
        ];

        let stats = store
            .reindex_file("src/a.rs", &chunks, &embedder, 42)
            .await
            .expect("reindex");

        assert_eq!(stats.chunks, 2);
        assert_eq!(stats.embedded, 2, "new file embeds all chunks");
        assert_eq!(stats.reused, 0);
        assert_eq!(embedder.texts_embedded(), 2);
        assert_eq!(count_for_path(&store, "src/a.rs").await, 2, "rows inserted");

        // Vector values preserved (CountingEmbedder => vec![content.len(); DIM]).
        let v = vector_for(&store, "src/a.rs", "h-a").await.expect("h-a");
        assert_eq!(v, vec!["fn a() {}".len() as f32; DIM]);
    }

    #[tokio::test]
    async fn reindex_file_unchanged_second_pass_reuses_all() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());
        let embedder = CountingEmbedder::new(DIM);

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        let chunks = vec![
            make_chunk("c1", "fn a() {}", "h-a", "src/a.rs", 1, 3, None),
            make_chunk("c2", "fn b() {}", "h-b", "src/a.rs", 4, 6, None),
        ];

        store
            .reindex_file("src/a.rs", &chunks, &embedder, 1)
            .await
            .expect("first reindex");
        assert_eq!(embedder.texts_embedded(), 2);

        // Second pass with identical chunks reuses every vector — zero embeds.
        let stats = store
            .reindex_file("src/a.rs", &chunks, &embedder, 2)
            .await
            .expect("second reindex");

        assert_eq!(stats.embedded, 0, "unchanged file embeds nothing");
        assert_eq!(stats.reused, 2);
        assert_eq!(stats.chunks, 2);
        assert_eq!(
            embedder.texts_embedded(),
            2,
            "no further embedding on unchanged second pass"
        );
        assert_eq!(
            count_for_path(&store, "src/a.rs").await,
            2,
            "no duplicate rows"
        );
    }

    #[tokio::test]
    async fn reindex_file_one_changed_chunk_embeds_only_that_chunk() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());
        let embedder = CountingEmbedder::new(DIM);

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        let v1 = vec![
            make_chunk("c1", "fn a() {}", "h-a", "src/a.rs", 1, 3, None),
            make_chunk("c2", "fn b() {}", "h-b", "src/a.rs", 4, 6, None),
        ];
        store
            .reindex_file("src/a.rs", &v1, &embedder, 1)
            .await
            .expect("first");
        assert_eq!(embedder.texts_embedded(), 2);

        // c2 changes content+hash; c1 unchanged.
        let v2 = vec![
            make_chunk("c1", "fn a() {}", "h-a", "src/a.rs", 1, 3, None),
            make_chunk("c2", "fn b_changed() {}", "h-b2", "src/a.rs", 4, 6, None),
        ];
        let stats = store
            .reindex_file("src/a.rs", &v2, &embedder, 2)
            .await
            .expect("second");

        assert_eq!(stats.embedded, 1, "only the changed chunk is embedded");
        assert_eq!(stats.reused, 1);
        assert_eq!(
            embedder.texts_embedded(),
            3,
            "two from first pass + one changed chunk"
        );
        assert_eq!(count_for_path(&store, "src/a.rs").await, 2);
        // The old hash is gone, the new hash is present (delete-then-insert).
        let map = store
            .existing_embeddings_by_content_hash("src/a.rs")
            .await
            .expect("read");
        assert!(map.contains_key("h-a"));
        assert!(map.contains_key("h-b2"));
        assert!(!map.contains_key("h-b"), "old chunk hash removed");
    }

    #[tokio::test]
    async fn reindex_file_empty_chunks_deletes_and_inserts_nothing() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());
        let embedder = CountingEmbedder::new(DIM);

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        let chunks = vec![make_chunk("c1", "fn a() {}", "h-a", "src/a.rs", 1, 3, None)];
        store
            .reindex_file("src/a.rs", &chunks, &embedder, 1)
            .await
            .expect("seed");
        assert_eq!(count_for_path(&store, "src/a.rs").await, 1);

        // Now reindex with NO chunks — old rows deleted, nothing inserted.
        let stats = store
            .reindex_file("src/a.rs", &[], &embedder, 2)
            .await
            .expect("empty");

        assert_eq!(stats.chunks, 0);
        assert_eq!(stats.embedded, 0);
        assert_eq!(stats.reused, 0);
        assert_eq!(
            count_for_path(&store, "src/a.rs").await,
            0,
            "old rows deleted"
        );
    }

    #[tokio::test]
    async fn reindex_file_inserts_more_than_one_batch() {
        // Exceed INSERT_BATCH (500) to prove batched inserts land every row.
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());
        let embedder = CountingEmbedder::new(DIM);

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        let total = INSERT_BATCH + 23; // 523 chunks -> 2 batches.
        let chunks: Vec<Chunk> = (0..total)
            .map(|i| {
                make_chunk(
                    &format!("c{i}"),
                    &format!("fn f{i}() {{}}"),
                    &format!("h-{i}"),
                    "src/big.rs",
                    i,
                    i + 1,
                    None,
                )
            })
            .collect();

        let stats = store
            .reindex_file("src/big.rs", &chunks, &embedder, 1)
            .await
            .expect("reindex big file");

        assert_eq!(stats.chunks, total);
        assert_eq!(stats.embedded, total);
        assert_eq!(
            count_for_path(&store, "src/big.rs").await,
            total,
            "all rows across both batches are inserted"
        );
    }

    // -----------------------------------------------------------------------
    // 3.8 — search tests
    // -----------------------------------------------------------------------
    //
    // Vector design: we use small dim=4 stores so vectors are easy to reason
    // about.  We plant rows with KNOWN vectors and query with a vector whose
    // L2 distance to each row is predictable.
    //
    // L2 distance (squared, what LanceDB returns by default):
    //   dist([q1..q4], [r1..r4]) = sum((qi - ri)^2)
    //
    // We use unit basis vectors so distances are integers and trivially orderable.

    const SDIM: usize = 4; // small dim for search tests
    const SMODEL: &str = "test-model-4d";

    /// Build a store with SDIM embedding dimension.
    async fn search_store(project: &Path, data_dir: &Path) -> VectorStore {
        VectorStore::new(project, &config_with_data_dir(data_dir), SDIM, SMODEL)
            .await
            .expect("create search store")
    }

    /// Make a `ChunkRow` with an explicit language, symbol_name/type, and vector.
    fn search_row(
        id: &str,
        rel_path: &str,
        language: &str,
        vector: Vec<f32>,
        symbol_name: Option<&str>,
        symbol_type: Option<&str>,
    ) -> ChunkRow {
        ChunkRow {
            id: id.to_string(),
            content_hash: format!("ch-{id}"),
            vector,
            rel_path: rel_path.to_string(),
            start_line: 1,
            end_line: 5,
            symbol_name: symbol_name.map(|s| s.to_string()),
            symbol_type: symbol_type.map(|s| s.to_string()),
            language: language.to_string(),
            content: format!("content of {id}"),
            last_modified: 1_700_000_000,
        }
    }

    // --- AC: top_k == 0 → empty list, no query ---

    #[tokio::test]
    async fn vector_store_search_top_k_zero_returns_empty() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let mut store = search_store(project.path(), data_dir.path()).await;

        // Seed some rows so the table is non-empty — search must still be a no-op.
        store
            .insert_chunks(&[search_row(
                "r1",
                "src/a.rs",
                "rust",
                vec![1.0, 0.0, 0.0, 0.0],
                None,
                None,
            )])
            .await
            .expect("seed");

        let results = store
            .search(&[1.0, 0.0, 0.0, 0.0], 0, None)
            .await
            .expect("search top_k=0 must succeed");

        assert!(results.is_empty(), "top_k=0 must return empty vec");
    }

    // --- AC: dimension mismatch → clear error, no LanceDB call ---

    #[tokio::test]
    async fn vector_store_search_dim_mismatch_returns_error() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let store = search_store(project.path(), data_dir.path()).await;

        // SDIM == 4, query with 3 elements — must fail fast with a clear message.
        let err = store
            .search(&[1.0, 0.0, 0.0], 5, None)
            .await
            .expect_err("dim mismatch must error");

        assert!(
            matches!(err, VektorError::Storage(_)),
            "expected Storage error, got: {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains('3') && msg.contains('4'),
            "error must mention actual ({}) and expected ({}) dims; got: {msg}",
            3,
            SDIM
        );
    }

    // --- AC: ranking — nearest first, all metadata correct ---

    #[tokio::test]
    async fn vector_store_search_ranking_nearest_first() {
        // Seed 3 rows with axis-aligned vectors of known L2 distances to the
        // query [1,0,0,0]:
        //   row "near":  [1,0,0,0]  → dist = 0  (exact match)
        //   row "mid":   [0,1,0,0]  → dist = 2  (two 1^2 = 2)
        //   row "far":   [0,0,0,1]  → dist = 2  ... actually same — let's use
        //
        // Use deliberate distances:
        //   query:      [1, 0, 0, 0]
        //   "near":     [1, 0, 0, 0]  → L2 = 0
        //   "mid":      [0, 1, 0, 0]  → L2 = 1^2 + 1^2 = 2
        //   "far":      [0, 0, 1, 0]  → same as mid... use a farther one
        //   "far":      [-1, 0, 0, 0] → L2 = (1-(-1))^2 = 4
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let mut store = search_store(project.path(), data_dir.path()).await;

        store
            .insert_chunks(&[
                search_row(
                    "near",
                    "src/near.rs",
                    "rust",
                    vec![1.0, 0.0, 0.0, 0.0],
                    Some("fn_near"),
                    Some("function"),
                ),
                search_row(
                    "mid",
                    "src/mid.rs",
                    "rust",
                    vec![0.0, 1.0, 0.0, 0.0],
                    None,
                    None,
                ),
                search_row(
                    "far",
                    "src/far.rs",
                    "rust",
                    vec![-1.0, 0.0, 0.0, 0.0],
                    None,
                    None,
                ),
            ])
            .await
            .expect("seed");

        let query = vec![1.0_f32, 0.0, 0.0, 0.0];
        let results = store
            .search(&query, 3, None)
            .await
            .expect("search must succeed");

        assert_eq!(results.len(), 3, "top_k=3 returns all 3 rows");

        // Nearest-first ordering.
        assert_eq!(results[0].id, "near", "nearest row first");
        assert_eq!(results[1].id, "mid", "mid-distance second");
        assert_eq!(results[2].id, "far", "farthest row last");

        // Scores are non-negative L2 distances, ascending.
        assert!(
            results[0].score <= results[1].score,
            "scores non-decreasing: {} <= {}",
            results[0].score,
            results[1].score
        );
        assert!(
            results[1].score <= results[2].score,
            "scores non-decreasing: {} <= {}",
            results[1].score,
            results[2].score
        );

        // Near row distance should be ~0.
        assert!(
            results[0].score < 1e-5,
            "exact match has distance ≈ 0; got {}",
            results[0].score
        );
    }

    // --- AC: all metadata fields extracted correctly ---

    #[tokio::test]
    async fn vector_store_search_all_metadata_fields_correct() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let mut store = search_store(project.path(), data_dir.path()).await;

        // One row with all fields populated.
        let mut row = search_row(
            "meta-row",
            "src/lib.rs",
            "python",
            vec![1.0, 0.0, 0.0, 0.0],
            Some("my_func"),
            Some("function"),
        );
        row.start_line = 10;
        row.end_line = 20;
        row.last_modified = 1_234_567_890;
        row.content = "def my_func(): pass".to_string();
        row.content_hash = "explicit-hash".to_string();

        store.insert_chunks(&[row]).await.expect("seed");

        let results = store
            .search(&[1.0, 0.0, 0.0, 0.0], 1, None)
            .await
            .expect("search");

        assert_eq!(results.len(), 1);
        let r = &results[0];

        assert_eq!(r.id, "meta-row");
        assert_eq!(r.content_hash, "explicit-hash");
        assert_eq!(r.rel_path, "src/lib.rs");
        assert_eq!(r.start_line, 10);
        assert_eq!(r.end_line, 20);
        assert_eq!(r.symbol_name, Some("my_func".to_string()));
        assert_eq!(r.symbol_type, Some("function".to_string()));
        assert_eq!(r.language, "python");
        assert_eq!(r.content, "def my_func(): pass");
        assert_eq!(r.last_modified, 1_234_567_890);
        assert!(r.score >= 0.0, "score must be non-negative");
    }

    #[tokio::test]
    async fn search_projection_excludes_vector_column() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let mut store = search_store(project.path(), data_dir.path()).await;

        store
            .insert_chunks(&[search_row(
                "projection-row",
                "src/projection.rs",
                "rust",
                vec![1.0, 0.0, 0.0, 0.0],
                Some("projected"),
                Some("function"),
            )])
            .await
            .expect("seed");

        let table = store.chunks_table().await.expect("open table");
        let stream = table
            .query()
            .nearest_to(&[1.0_f32, 0.0, 0.0, 0.0])
            .expect("nearest_to")
            .limit(1)
            .select(search_result_projection())
            .execute()
            .await
            .expect("execute projection query");
        let batches: Vec<RecordBatch> = stream.try_collect().await.expect("collect batches");
        let batch = batches.first().expect("one projected batch");

        assert!(
            batch.column_by_name("vector").is_none(),
            "stored vector column must not be materialized on the search hot path"
        );
        assert!(
            batch.column_by_name("_distance").is_some(),
            "_distance must survive LanceDB scoring auto-projection"
        );
        assert!(batch.column_by_name("content").is_some());

        let results = store
            .search(&[1.0, 0.0, 0.0, 0.0], 1, None)
            .await
            .expect("projected search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "projection-row");
    }

    #[tokio::test]
    async fn chunks_by_ids_hydrates_content_without_vector_column() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let mut store = search_store(project.path(), data_dir.path()).await;
        let mut row = search_row(
            "hydrate-row",
            "src/hydrate.rs",
            "rust",
            vec![1.0, 0.0, 0.0, 0.0],
            Some("hydrate"),
            Some("function"),
        );
        row.content = "fn hydrate() {}".to_string();

        store.insert_chunks(&[row]).await.expect("seed");

        let rows = store
            .chunks_by_ids(&["hydrate-row".to_string(), "missing-row".to_string()])
            .await
            .expect("hydrate by id");

        assert_eq!(rows.len(), 1);
        let hydrated = rows.get("hydrate-row").expect("hydrated row");
        assert_eq!(hydrated.content, "fn hydrate() {}");
        assert_eq!(hydrated.rel_path, "src/hydrate.rs");
        assert_eq!(hydrated.symbol_name, Some("hydrate".to_string()));
        assert_eq!(hydrated.symbol_type, Some("function".to_string()));
        assert_eq!(hydrated.score, 0.0);
    }

    // --- AC: null symbol_name / symbol_type round-trip correctly ---

    #[tokio::test]
    async fn vector_store_search_null_symbol_fields_round_trip() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let mut store = search_store(project.path(), data_dir.path()).await;

        store
            .insert_chunks(&[search_row(
                "no-sym",
                "src/a.rs",
                "rust",
                vec![1.0, 0.0, 0.0, 0.0],
                None, // symbol_name: null
                None, // symbol_type: null
            )])
            .await
            .expect("seed");

        let results = store
            .search(&[1.0, 0.0, 0.0, 0.0], 1, None)
            .await
            .expect("search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].symbol_name, None, "null symbol_name preserved");
        assert_eq!(results[0].symbol_type, None, "null symbol_type preserved");
    }

    // --- AC: top_k limits results ---

    #[tokio::test]
    async fn vector_store_search_top_k_limits_results() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let mut store = search_store(project.path(), data_dir.path()).await;

        // Seed 5 rows.
        let rows: Vec<ChunkRow> = (0..5_u32)
            .map(|i| {
                // Spread vectors along first axis so distances are distinct.
                let v = vec![i as f32, 0.0, 0.0, 0.0];
                search_row(
                    &format!("r{i}"),
                    &format!("src/{i}.rs"),
                    "rust",
                    v,
                    None,
                    None,
                )
            })
            .collect();
        store.insert_chunks(&rows).await.expect("seed");

        let results = store
            .search(&[0.0, 0.0, 0.0, 0.0], 3, None)
            .await
            .expect("search");

        assert_eq!(results.len(), 3, "top_k=3 caps at 3 results");
    }

    // --- AC: filter narrows results at the LanceDB query layer ---

    #[tokio::test]
    async fn vector_store_search_filter_by_language() {
        // Seed rust + python rows with same direction but different languages.
        // A filter for "language = 'rust'" must return only rust rows.
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let mut store = search_store(project.path(), data_dir.path()).await;

        store
            .insert_chunks(&[
                search_row(
                    "rs1",
                    "src/a.rs",
                    "rust",
                    vec![1.0, 0.0, 0.0, 0.0],
                    None,
                    None,
                ),
                search_row(
                    "py1",
                    "src/b.py",
                    "python",
                    vec![1.0, 0.0, 0.0, 0.0],
                    None,
                    None,
                ),
                search_row(
                    "rs2",
                    "src/c.rs",
                    "rust",
                    vec![0.0, 1.0, 0.0, 0.0],
                    None,
                    None,
                ),
            ])
            .await
            .expect("seed");

        let results = store
            .search(&[1.0, 0.0, 0.0, 0.0], 10, Some("language = 'rust'"))
            .await
            .expect("search with filter");

        assert_eq!(results.len(), 2, "filter keeps only rust rows");
        for r in &results {
            assert_eq!(r.language, "rust", "all results are rust");
        }
    }

    #[tokio::test]
    async fn vector_store_search_filter_by_rel_path() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let mut store = search_store(project.path(), data_dir.path()).await;

        store
            .insert_chunks(&[
                search_row(
                    "a1",
                    "src/a.rs",
                    "rust",
                    vec![1.0, 0.0, 0.0, 0.0],
                    None,
                    None,
                ),
                search_row(
                    "b1",
                    "src/b.rs",
                    "rust",
                    vec![1.0, 0.0, 0.0, 0.0],
                    None,
                    None,
                ),
            ])
            .await
            .expect("seed");

        let predicate = format!("rel_path = '{}'", escape_sql_string_literal("src/a.rs"));
        let results = store
            .search(&[1.0, 0.0, 0.0, 0.0], 10, Some(&predicate))
            .await
            .expect("search with path filter");

        assert_eq!(results.len(), 1, "filter keeps only src/a.rs");
        assert_eq!(results[0].rel_path, "src/a.rs");
    }

    // -----------------------------------------------------------------------
    // 6.x — batched multi-file reindex (`reindex_files`) + `optimize`
    // -----------------------------------------------------------------------

    /// Count Lance version manifests under the store's `lance/` dir. Every
    /// LanceDB write (delete/insert/compact) commits one new version, so this
    /// is the observable proxy for "how many write transactions happened".
    fn manifest_count(dir: &Path) -> usize {
        let mut count = 0;
        let Ok(entries) = fs::read_dir(dir) else {
            return 0;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += manifest_count(&path);
            } else if path.extension().is_some_and(|e| e == "manifest") {
                count += 1;
            }
        }
        count
    }

    fn batch_files(count: usize) -> Vec<(String, Vec<Chunk>)> {
        (0..count)
            .map(|i| {
                let rel_path = format!("src/file{i}.rs");
                let chunks = vec![
                    make_chunk(
                        &format!("f{i}-c1"),
                        &format!("fn alpha{i}() {{}}"),
                        &format!("h-{i}-alpha"),
                        &rel_path,
                        1,
                        3,
                        Some(Language::Rust),
                    ),
                    make_chunk(
                        &format!("f{i}-c2"),
                        &format!("fn beta{i}() {{}}"),
                        &format!("h-{i}-beta"),
                        &rel_path,
                        4,
                        6,
                        Some(Language::Rust),
                    ),
                ];
                (rel_path, chunks)
            })
            .collect()
    }

    #[tokio::test]
    async fn reindex_files_batch_bounds_lance_versions_and_inserts_all_rows() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());
        let embedder = CountingEmbedder::new(DIM);

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        let owned = batch_files(10);
        let files: Vec<FileToReindex<'_>> = owned
            .iter()
            .map(|(rel_path, chunks)| FileToReindex {
                rel_path,
                chunks,
                last_modified: 42,
            })
            .collect();

        let stats = store
            .reindex_files(&files, &embedder)
            .await
            .expect("batch reindex");

        assert_eq!(stats.chunks, 20, "one row per input chunk");
        assert_eq!(stats.embedded, 20, "all chunks are new");
        assert_eq!(stats.reused, 0);
        assert_eq!(embedder.texts_embedded(), 20);
        assert_eq!(count_for_path(&store, "src/file0.rs").await, 2);
        assert_eq!(count_for_path(&store, "src/file9.rs").await, 2);

        // The whole batch must land in a bounded number of Lance write
        // transactions (create table + one delete + bounded inserts), NOT
        // one delete+insert pair per file (~20+ versions for 10 files).
        let versions = manifest_count(store.lance_dir());
        assert!(
            versions <= 5,
            "expected a bounded number of Lance versions for a 10-file batch, got {versions}"
        );
    }

    #[tokio::test]
    async fn reindex_files_second_pass_reuses_all_vectors() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());
        let embedder = CountingEmbedder::new(DIM);

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        let owned = batch_files(3);
        let files: Vec<FileToReindex<'_>> = owned
            .iter()
            .map(|(rel_path, chunks)| FileToReindex {
                rel_path,
                chunks,
                last_modified: 1,
            })
            .collect();

        store
            .reindex_files(&files, &embedder)
            .await
            .expect("first batch reindex");
        assert_eq!(embedder.texts_embedded(), 6);

        let stats = store
            .reindex_files(&files, &embedder)
            .await
            .expect("second batch reindex");

        assert_eq!(stats.embedded, 0, "unchanged batch embeds nothing");
        assert_eq!(stats.reused, 6);
        assert_eq!(
            embedder.texts_embedded(),
            6,
            "no further embedder texts on unchanged second pass"
        );
        assert_eq!(count_for_path(&store, "src/file1.rs").await, 2);
    }

    #[tokio::test]
    async fn reindex_files_embeds_identical_new_content_once_across_files() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());
        let embedder = CountingEmbedder::new(DIM);

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        // Two different files containing IDENTICAL content (same content_hash).
        // Content-addressed reuse means one embedding serves both rows.
        let chunks_a = vec![make_chunk(
            "a-c1",
            "fn same() {}",
            "h-same",
            "src/a.rs",
            1,
            3,
            None,
        )];
        let chunks_b = vec![make_chunk(
            "b-c1",
            "fn same() {}",
            "h-same",
            "src/b.rs",
            1,
            3,
            None,
        )];
        let files = vec![
            FileToReindex {
                rel_path: "src/a.rs",
                chunks: &chunks_a,
                last_modified: 1,
            },
            FileToReindex {
                rel_path: "src/b.rs",
                chunks: &chunks_b,
                last_modified: 1,
            },
        ];

        let stats = store
            .reindex_files(&files, &embedder)
            .await
            .expect("batch reindex");

        assert_eq!(stats.chunks, 2, "one row per file");
        assert_eq!(
            embedder.texts_embedded(),
            1,
            "identical content across the batch embeds exactly once"
        );
        assert_eq!(count_for_path(&store, "src/a.rs").await, 1);
        assert_eq!(count_for_path(&store, "src/b.rs").await, 1);
    }

    #[tokio::test]
    async fn reindex_files_empty_chunks_still_deletes_stale_rows() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());
        let embedder = CountingEmbedder::new(DIM);

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        let chunks = vec![make_chunk(
            "c1",
            "fn gone() {}",
            "h-gone",
            "src/a.rs",
            1,
            3,
            None,
        )];
        store
            .reindex_file("src/a.rs", &chunks, &embedder, 1)
            .await
            .expect("seed rows");
        assert_eq!(count_for_path(&store, "src/a.rs").await, 1);

        // The file emptied (e.g. all chunks became secret-only): its stale
        // rows must be deleted even though nothing is inserted.
        let files = vec![FileToReindex {
            rel_path: "src/a.rs",
            chunks: &[],
            last_modified: 2,
        }];
        let stats = store
            .reindex_files(&files, &embedder)
            .await
            .expect("empty batch reindex");

        assert_eq!(stats.chunks, 0);
        assert_eq!(
            count_for_path(&store, "src/a.rs").await,
            0,
            "stale rows gone"
        );
    }

    #[tokio::test]
    async fn optimize_compacts_fragments_and_prunes_old_versions() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());
        let embedder = CountingEmbedder::new(DIM);

        let mut store = VectorStore::new(project.path(), &config, DIM, MODEL)
            .await
            .expect("create store");

        // Per-file reindexing accrues one version per delete/insert — the
        // pathological pattern optimize() must clean up after.
        let owned = batch_files(6);
        for (rel_path, chunks) in &owned {
            store
                .reindex_file(rel_path, chunks, &embedder, 1)
                .await
                .expect("per-file reindex");
        }
        let versions_before = manifest_count(store.lance_dir());
        assert!(
            versions_before >= 7,
            "per-file writes should accrue many versions, got {versions_before}"
        );

        store.optimize().await.expect("optimize");

        let versions_after = manifest_count(store.lance_dir());
        assert!(
            versions_after < versions_before,
            "optimize must prune old versions ({versions_before} -> {versions_after})"
        );

        // Data must survive compaction + pruning intact.
        for i in 0..6 {
            assert_eq!(
                count_for_path(&store, &format!("src/file{i}.rs")).await,
                2,
                "rows intact after optimize (file{i})"
            );
        }
    }
}
