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

## Acceptance Criteria

- [ ] Extracts function/class/impl/struct chunks using the PRD node-type list
- [ ] Populates `Chunk` metadata: content hash, rel path, line range, symbol name/type, language
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
