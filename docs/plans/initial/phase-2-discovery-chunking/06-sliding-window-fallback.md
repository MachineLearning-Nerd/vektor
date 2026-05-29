# Task 2.6 — sliding-window fallback

**Phase**: 2 — Discovery + Chunking
**Task ID**: 2.6
**PRD reference**: Section 12 Function 2.4 (`extract_chunks_sliding`)
**Roadmap stage**: Stage 2 / `v0.2.0`
**Effort estimate**: M
**Depends on**: 1.7b
**Blocks**: 2.7

## Objective

Implement line-based fallback chunking for unsupported languages and docs. Code files use 80-line windows with 25% overlap; markdown/text/rst use 40-line windows with 40% overlap unless config overrides require otherwise.

## Acceptance Criteria

- [ ] Produces deterministic overlapping chunks for arbitrary text
- [ ] Handles files shorter than one window
- [ ] Handles empty files without panics
- [ ] Uses doc-specific defaults for `.md`, `.txt`, and `.rst`
- [ ] Tests cover overlap math and boundary conditions

## Verification

```bash
cargo test sliding
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- The PRD text still mentions 80-line fallback in one place; the Phase 2 README refines docs to 40-line windows with 40% overlap.
