# Task 2.3 — language detection

**Phase**: 2 — Discovery + Chunking
**Task ID**: 2.3
**PRD reference**: Section 12 Function 2.1 (`detect_language`)
**Roadmap stage**: Stage 2 / `v0.2.0`
**Effort estimate**: S
**Depends on**: 1.7b
**Blocks**: 2.4

## Objective

Define Vektor's Phase 2 language enum and implement `detect_language(path) -> Option<Language>` for Python, TypeScript, JavaScript, Rust, and Go. Unsupported files return `None` for sliding-window fallback.

## Acceptance Criteria

- [ ] Recognizes `.py`, `.ts`, `.tsx`, `.js`, `.jsx`, `.rs`, and `.go`
- [ ] Distinguishes TypeScript from JavaScript when needed by tree-sitter grammar setup
- [ ] Returns `None` for unsupported extensions and extensionless files
- [ ] Has table-driven tests

## Verification

```bash
cargo test language
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Keep this module small and stable; downstream parser/chunker tasks should depend on the enum rather than rechecking extensions.
