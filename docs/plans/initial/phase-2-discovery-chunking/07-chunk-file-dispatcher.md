# Task 2.7 — chunk_file dispatcher

**Phase**: 2 — Discovery + Chunking
**Task ID**: 2.7
**PRD reference**: Section 12 Function 2.5 (`chunk_file`)
**Roadmap stage**: Stage 2 / `v0.2.0`
**Effort estimate**: M
**Depends on**: 2.5, 2.6
**Blocks**: 2.8

## Objective

Implement the top-level chunking dispatcher. It detects language, attempts AST chunking for supported languages, and falls back to sliding windows for unsupported or parse-failed files.

## Acceptance Criteria

- [ ] `chunk_file(path, content, config)` returns `Vec<Chunk>` for every readable file type
- [ ] Supported languages use AST chunks when parsing succeeds
- [ ] Unsupported languages and parse failures use sliding-window chunks with a debug log
- [ ] Chunk IDs/content hashes are stable across repeated runs on the same content
- [ ] Tests cover AST path, fallback path, and parse-failure fallback

## Verification

```bash
cargo test chunk_file
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Do not write chunks to vector storage here. Phase 3 owns storage.
