# Task 2.3 — chunk types + language detection

**Phase**: 2 — Discovery + Chunking
**Task ID**: 2.3
**PRD reference**: Section 12 Function 2.1 (`detect_language`)
**Roadmap stage**: Stage 2 / `v0.2.0`
**Effort estimate**: S
**Depends on**: 1.7b
**Blocks**: 2.4, 2.6

## Objective

Define Vektor's shared Phase 2 chunk metadata type, language enum, and `detect_language(path) -> Option<Language>` for Python, TypeScript, JavaScript, Rust, and Go. Unsupported files return `None` for sliding-window fallback.

## Inputs (must exist before starting)

- Phase 1 crate/module layout
- PRD Section 12 `Chunk` metadata contract
- Tree-sitter language dependencies already pinned for the supported languages

## Outputs (must exist after completion)

- Shared `Chunk` type used by AST chunking, sliding fallback, dispatcher, and dump CLI
- Shared `Language` enum for supported AST languages
- Table-driven tests for extension mapping

## Approach

- Put shared chunking types in a module that both AST and fallback chunkers can import.
- Keep language detection extension-based and deterministic.
- Preserve a distinct enum variant for TypeScript/TSX versus JavaScript/JSX so parser setup can choose the right grammar.

## Acceptance criteria

- [ ] Defines the shared `Chunk` metadata fields from PRD Section 12: content, full SHA-256 content hash, relative path, line range, symbol name/type, and language
- [ ] Recognizes `.py`, `.ts`, `.tsx`, `.js`, `.jsx`, `.rs`, and `.go`
- [ ] Distinguishes TypeScript from JavaScript when needed by tree-sitter grammar setup
- [ ] Returns `None` for unsupported extensions and extensionless files
- [ ] Has table-driven tests

## Verification

```bash
cargo test chunk_types
cargo test language
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Keep this module small and stable; downstream parser/chunker tasks should depend on the enum rather than rechecking extensions.
