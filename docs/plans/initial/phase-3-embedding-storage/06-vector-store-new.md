# Task 3.6 — VectorStore initialization and schema

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.6
**PRD reference**: Section 4.10 storage schema, Section 12 Function 3.6 (`VectorStore::new`)
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: M
**Depends on**: 2.9
**Blocks**: 3.7a, 3.9

## Objective

Create the embedded LanceDB vector store at a project-scoped path, initialize the chunks table with the PRD Arrow schema, and create metadata/stat tables needed for model-dimension migration and ANN index churn tracking.

## Inputs (must exist before starting)

- Phase 2 project-root handling and `HashStore` project scoping
- `lancedb = "0.29"` dependency in `Cargo.toml`
- PRD Section 4.10 LanceDB schema and metadata/stat requirements

## Outputs (must exist after completion)

- `src/vector_store/mod.rs` exported from `src/main.rs`
- `VectorStore::new(project_root: &Path, config: &Config, dim: usize, model_name: &str) -> Result<Self>`
- LanceDB data path under `config.index.data_dir/<project-hash>/lance/`
- Chunks table with `id`, `content_hash`, `vector`, `rel_path`, `start_line`, `end_line`, `symbol_name`, `symbol_type`, `language`, `content`, and `last_modified`
- Metadata/stat persistence for `model_name`, `embedding_dim`, `last_full_index_at`, `vektor_version`, `chunks_at_last_ann_rebuild`, `chunks_inserted_since`, and `chunks_deleted_since`

## Approach

- Reuse the canonicalized project-root hash already used for `state.db` so project data stays colocated.
- Let `lancedb` own Arrow crate versions transitively; do not add direct Arrow pins unless the Rust API requires them.
- Store vector dimension in metadata and reject dimension mismatches with a clear re-index-required message.
- Keep ANN index building out of this task unless table creation requires a harmless empty-table setup call; insertion/search tasks can trigger index maintenance after data exists.
- Add tests that inspect schema/metadata using a temporary project and data directory.

## Acceptance criteria

- [ ] `VectorStore::new` creates the project-scoped `lance/` directory
- [ ] Chunks table schema matches the Phase 3 README and PRD field list
- [ ] Vector column uses fixed-size float vectors with the configured dimension
- [ ] Metadata records model name and embedding dimension
- [ ] Reopening the same store with the same model/dim succeeds
- [ ] Reopening with a different dim returns a clear re-index-required error
- [ ] Tests isolate Vektor data under temp directories

## Verification

```bash
cargo test vector_store
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Do not implement reindex, search, or delete behavior in this task.
- If the LanceDB Rust API requires async initialization, keep `VectorStore::new` async and update downstream task signatures consistently.
