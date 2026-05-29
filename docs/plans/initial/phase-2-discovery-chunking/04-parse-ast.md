# Task 2.4 — parse_ast

**Phase**: 2 — Discovery + Chunking
**Task ID**: 2.4
**PRD reference**: Section 12 Function 2.2 (`parse_ast`)
**Roadmap stage**: Stage 2 / `v0.2.0`
**Effort estimate**: S
**Depends on**: 2.3
**Blocks**: 2.5

## Objective

Construct tree-sitter parsers for each supported `Language` and parse source text into a `tree_sitter::Tree`.

## Inputs (must exist before starting)

- Shared `Language` enum from task 2.3
- Tree-sitter grammar crates and versions pinned in `Cargo.lock`

## Outputs (must exist after completion)

- Parser helper that maps each supported language to the current tree-sitter grammar API
- Tests proving parser setup works across the supported language families

## Approach

- Build parser setup behind one function so grammar API differences stay localized.
- Convert parser setup failures and `parse` returning no tree into `VektorError`.
- Preserve syntax-error information on the returned tree so task 2.7 can choose fallback behavior.

## Acceptance criteria

- [ ] Parser construction works for all 5 supported language families
- [ ] Parser setup failures and missing parse trees become `crate::error::VektorError` variants, not panics
- [ ] Syntax-error trees are detectable by callers
- [ ] Tests parse small valid fixtures per language
- [ ] Invalid or unsupported parser setup returns an error with context

## Verification

```bash
cargo test parse_ast
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Tree-sitter language crate APIs vary by version. Verify against the versions currently pinned in `Cargo.lock`.
