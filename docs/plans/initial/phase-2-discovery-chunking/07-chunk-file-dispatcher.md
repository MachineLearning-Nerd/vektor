# Task 2.7 — chunk_file dispatcher

**Phase**: 2 — Discovery + Chunking
**Task ID**: 2.7
**PRD reference**: Section 12 Function 2.5 (`chunk_file`)
**Roadmap stage**: Stage 2 / `v0.2.0`
**Effort estimate**: M
**Depends on**: 2.2, 2.5, 2.6
**Blocks**: 2.8

## Objective

Implement the top-level chunking dispatcher. It detects language, attempts AST chunking for supported languages, and falls back to sliding windows for unsupported or parse-failed files.

## Inputs (must exist before starting)

- Full SHA-256 content-hash helper from task 2.2
- AST chunker from task 2.5
- Sliding-window fallback from task 2.6

## Outputs (must exist after completion)

- `chunk_file(path, content, config)` dispatcher used by the dump CLI and future indexer
- Tests covering AST, unsupported-file fallback, and syntax-error fallback paths

## Approach

- Detect language once, dispatch supported languages to AST chunking, and route unsupported files directly to sliding windows.
- If parser setup fails or the parsed tree is unsuitable for semantic chunking, fall back to sliding windows with a debug log.
- Use the full content-hash helper for chunk `content_hash` and content-addressed chunk IDs; keep the truncated file hash limited to HashStore change detection.

## Acceptance criteria

- [ ] `chunk_file(path, content, config)` returns `Vec<Chunk>` for every readable file type
- [ ] Supported languages use AST chunks when parsing succeeds
- [ ] Unsupported languages, parser failures, and syntax-error fallback cases use sliding-window chunks with a debug log
- [ ] Chunk IDs/full content hashes are stable across repeated runs on the same content
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
