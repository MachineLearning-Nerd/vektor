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
    collections::HashMap,
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
///   nulls. 3.7c must carry that mapping.
///
/// `#[allow(dead_code)]`: only the test seed-path constructs these until 3.7a/3.7c
/// wire up the real insert; allow until then.
#[allow(dead_code)]
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
///
/// `#[allow(dead_code)]`: consumed only via [`VectorStore::delete_by_file`] and
/// its tests until 3.7c routes deletes through it; allow until then.
#[allow(dead_code)]
fn escape_sql_string_literal(value: &str) -> String {
    value.replace('\'', "''")
}

/// Embedded LanceDB vector store for a single project.
///
/// `#[allow(dead_code)]`: used by 3.7a/3.7c (insert/reindex); allow until then.
/// Tests construct it, but the bin target sees the type and accessors as dead.
#[allow(dead_code)]
pub struct VectorStore {
    conn: Connection,
    lance_dir: PathBuf,
    meta_path: PathBuf,
    meta: StoreMeta,
}

#[allow(dead_code)]
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

    /// Persisted metadata for this store.
    pub fn meta(&self) -> &StoreMeta {
        &self.meta
    }

    /// Directory holding the LanceDB dataset (`<project-dir>/lance/`).
    pub fn lance_dir(&self) -> &Path {
        &self.lance_dir
    }

    /// Name of the chunks table.
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
    ///
    /// `#[allow(dead_code)]`: no non-test caller until 3.7a/3.7c; allow until then.
    #[allow(dead_code)]
    pub(crate) async fn insert_chunks(&self, rows: &[ChunkRow]) -> Result<()> {
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
    /// `#[allow(dead_code)]`: used by 3.7b reindex planning; allow until then.
    #[allow(dead_code)]
    pub(crate) async fn existing_embeddings_by_content_hash(
        &self,
        rel_path: &str,
    ) -> Result<HashMap<String, Vec<f32>>> {
        let predicate = format!("rel_path = '{}'", escape_sql_string_literal(rel_path));

        let table = self.chunks_table().await?;
        let stream = table
            .query()
            .only_if(predicate)
            // Read only the two columns the cache needs (content reuse key +
            // the vector to reuse). Selecting fewer columns is the documented
            // best practice for LanceDB's columnar reads.
            .select(Select::Columns(vec![
                "content_hash".to_string(),
                "vector".to_string(),
            ]))
            .execute()
            .await
            .map_err(|e| VektorError::Storage(e.to_string()))?;

        // Collect the result stream into in-memory batches. `SendableRecordBatchStream`
        // is `Stream<Item = Result<RecordBatch>>`, so `try_collect` yields the
        // batches or short-circuits on the first storage error.
        let batches: Vec<RecordBatch> = stream
            .try_collect()
            .await
            .map_err(|e| VektorError::Storage(e.to_string()))?;

        let mut out: HashMap<String, Vec<f32>> = HashMap::new();
        for batch in &batches {
            // Extract the two selected columns by NAME (not position): `select`
            // returns columns in the requested order, but reading by name is
            // robust to that and self-documenting.
            let hashes = batch
                .column_by_name("content_hash")
                .ok_or_else(|| {
                    VektorError::Storage("query result missing content_hash column".into())
                })?
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
                    .ok_or_else(|| {
                        VektorError::Storage("vector list items are not Float32".into())
                    })?;
                let vector: Vec<f32> = floats.values().to_vec();

                // First write wins for duplicate content hashes within this file;
                // identical content ⇒ identical embedding, so either is fine.
                match out.entry(content_hash) {
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert(vector);
                    }
                    std::collections::hash_map::Entry::Occupied(e) => {
                        tracing::debug!(
                            rel_path,
                            content_hash = e.key().as_str(),
                            "duplicate content_hash within file; keeping first vector"
                        );
                    }
                }
            }
        }

        Ok(out)
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
    /// `#[allow(dead_code)]`: no non-test caller until 3.7b/3.7c; allow until then.
    #[allow(dead_code)]
    pub(crate) async fn delete_by_file(&mut self, rel_path: &str) -> Result<usize> {
        let predicate = format!("rel_path = '{}'", escape_sql_string_literal(rel_path));

        let table = self.chunks_table().await?;
        let result = table
            .delete(&predicate)
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

        let store = VectorStore::new(project.path(), &config, DIM, MODEL)
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

        let store = VectorStore::new(project.path(), &config, DIM, MODEL)
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

        let store = VectorStore::new(project.path(), &config, DIM, MODEL)
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

        let store = VectorStore::new(project.path(), &config, DIM, MODEL)
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

        let store = VectorStore::new(project.path(), &config, DIM, MODEL)
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
}
