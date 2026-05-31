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
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use lancedb::{
    Connection,
    arrow::arrow_schema::{DataType, Field, Schema, SchemaRef},
    connect,
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

    fn save(&self, path: &Path) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| VektorError::Storage(format!("failed to serialize metadata: {e}")))?;
        fs::write(path, bytes)?;
        Ok(())
    }
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
