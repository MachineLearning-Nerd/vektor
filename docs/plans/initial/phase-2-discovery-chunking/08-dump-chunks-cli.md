# Task 2.8 — index --dump-chunks CLI

**Phase**: 2 — Discovery + Chunking
**Task ID**: 2.8
**PRD reference**: Stage 2 `v0.2.0` debug/inspection UX
**Roadmap stage**: Stage 2 / `v0.2.0`
**Effort estimate**: M
**Depends on**: 2.2, 2.7
**Blocks**: 2.9

## Objective

Replace the Phase 1 `vektor index` stub with Phase 2 behavior for `--dump-chunks`: chunk one file or discovered files under a directory and pretty-print chunk metadata/content for inspection.

## Acceptance Criteria

- [ ] `vektor index --dump-chunks <file>` prints chunks and exits 0
- [ ] `vektor index --dump-chunks <dir>` discovers files and prints chunks deterministically
- [ ] Plain `vektor index <path>` still avoids embedding/vector writes in Phase 2
- [ ] Output includes path, line range, language, symbol name/type, and content hash
- [ ] Integration tests use isolated `HOME`/`USERPROFILE`

## Verification

```bash
cargo test dump_chunks
cargo build
FAKE_HOME=$(mktemp -d)
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" ./target/debug/vektor index --dump-chunks src/
rm -rf "$FAKE_HOME"
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Keep stdout user-facing for this one-shot command. MCP stdout cleanliness only applies to `vektor serve`.
