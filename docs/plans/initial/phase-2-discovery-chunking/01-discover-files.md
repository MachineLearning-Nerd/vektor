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

## Inputs (must exist before starting)

- Phase 1 `vektor index` CLI stub and `Config` module
- `ignore` crate dependency from the project manifest
- `config.index.max_file_size_kb` default from PRD Section 6.3

## Outputs (must exist after completion)

- File-discovery module callable by later Phase 2 indexing tasks
- Tests proving gitignore handling, skip-list handling, max-size filtering, and deterministic ordering

## Approach

- Use `ignore::WalkBuilder` rooted at the requested directory.
- Layer Vektor's generated/cache directory skips on top of the ignore crate's gitignore handling.
- Filter by configured max file size before returning paths.
- Normalize returned paths into a deterministic sorted order.

## Acceptance criteria

- [ ] Walks files under a root path with nested `.gitignore` support
- [ ] Skips `.git`, `node_modules`, `__pycache__`, `target`, `dist`, `build`, and similar generated/cache outputs
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
