# Task 2.5 — AST chunker for 5 languages

**Phase**: 2 — Discovery + Chunking
**Task ID**: 2.5
**PRD reference**: Section 12 Function 2.3 (`extract_chunks_ast`)
**Roadmap stage**: Stage 2 / `v0.2.0`
**Effort estimate**: L
**Depends on**: 2.4
**Blocks**: 2.7

## Objective

Extract semantic chunks from tree-sitter ASTs for Python, TypeScript, JavaScript, Rust, and Go, including symbol metadata and header preservation for oversized chunks.

## Inputs (must exist before starting)

- Shared `Chunk` and `Language` types from task 2.3
- `parse_ast` helper from task 2.4
- PRD node-type list and chunk sizing defaults

## Outputs (must exist after completion)

- AST chunker for Python, TypeScript, JavaScript, Rust, and Go
- Fixture tests for supported language node extraction and oversized-node sub-chunking

## Approach

- Traverse language-specific tree-sitter nodes using the PRD node-type list, verified against the pinned grammar versions.
- Prefer method/function-level chunks inside large containers; avoid duplicate whole-container chunks when child chunks already represent the useful code.
- Split oversized AST chunks at `config.index.chunk_max_lines` with overlap and parent-signature/header preservation.
- Populate the shared `Chunk` metadata without writing to any storage layer.

## Acceptance criteria

- [ ] Extracts semantic chunks using the PRD node-type list, with Python classes and Rust impl blocks chunked at method/function level where possible
- [ ] Populates `Chunk` metadata: full content hash, rel path, line range, symbol name/type, language
- [ ] Splits chunks over `config.index.chunk_max_lines`
- [ ] Prepends parent signature/header to every sub-chunk after the first
- [ ] Fixture tests cover all supported languages and at least one oversized function

## Verification

```bash
cargo test ast_chunk
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Avoid byte/line off-by-one drift: tests should assert exact start/end lines.
- Keep fallback chunking out of this task; it belongs to 2.6.
