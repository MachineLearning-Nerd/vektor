# Task 2.2 — HashStore + SQLite state.db

**Phase**: 2 — Discovery + Chunking
**Task ID**: 2.2
**PRD reference**: Section 12 Function 1.3 (`hash_file`) and Function 1.4 (`HashStore`)
**Roadmap stage**: Stage 2 / `v0.2.0`
**Effort estimate**: M
**Depends on**: 2.1
**Blocks**: 2.8

## Objective

Add SHA-256 file hashing and a SQLite-backed `HashStore` for incremental indexing state. This is the only persisted Phase 2 state; no embeddings or vector store writes happen yet.

## Acceptance Criteria

- [ ] `hash_file(path) -> Result<String>` reads bytes and returns the first 16 hex chars of SHA-256
- [ ] `HashStore` creates `file_hashes(rel_path TEXT PRIMARY KEY, hash TEXT, status TEXT DEFAULT 'pending', indexed_at INTEGER)`
- [ ] `get_hash`, `set_hash`, `is_changed`, and `get_pending` are covered by tests
- [ ] State DB lives under the configured Vektor data directory
- [ ] Crash-recovery pending rows are preserved and returned deterministically

## Verification

```bash
cargo test hash_store
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Use `rusqlite` already pinned in `Cargo.toml`.
- Do not create LanceDB tables in this task; vector storage is Phase 3.
