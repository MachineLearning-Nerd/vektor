# Task 2.1 — discover_files

**Phase**: 2 — Discovery + Chunking
**Task ID**: 2.1
**PRD reference**: Section 12 Function 1.2 (`discover_files(root, config)`)
**Roadmap stage**: Stage 2 / `v0.2.0`
**Effort estimate**: M
**Depends on**: 1.7b
**Blocks**: 2.2, 3.10

## Objective

Implement `discover_files(root, config) -> Vec<PathBuf>` using the `ignore` crate. It must respect `.gitignore`, skip obvious generated/cache directories, enforce `config.index.max_file_size_kb`, and return deterministic sorted paths.

## Acceptance Criteria

- [ ] Walks files under a root path with nested `.gitignore` support
- [ ] Skips `.git`, `node_modules`, `__pycache__`, and build outputs
- [ ] Skips files larger than `config.index.max_file_size_kb`
- [ ] Returns sorted paths for deterministic tests
- [ ] Has unit/integration tests using a temp fixture repo

## Verification

```bash
cargo test discover_files
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Do not hand-roll `.gitignore` matching. Use `ignore::WalkBuilder`.
- Secret-file skip names are Phase 3 task 3.10 scope. Keep this task limited to general discovery and generated/cache exclusions so `v0.2.0` does not accidentally claim the `v0.3.0` secret-aware indexing behavior.
