# Task 2.8 — index CLI Phase 2 behavior

**Phase**: 2 — Discovery + Chunking
**Task ID**: 2.8
**PRD reference**: Stage 2 `v0.2.0` debug/inspection UX
**Roadmap stage**: Stage 2 / `v0.2.0`
**Effort estimate**: M
**Depends on**: 2.2, 2.7
**Blocks**: 2.9

## Objective

Replace the Phase 1 `vektor index` stub with Phase 2 behavior: `--dump-chunks` chunks one file or discovered files under a directory and pretty-prints chunk metadata/content for inspection, while plain `vektor index <path>` runs discovery/hash/chunking and updates HashStore state without embedding or vector writes.

## Inputs (must exist before starting)

- HashStore from task 2.2
- Chunk dispatcher from task 2.7
- Existing `IndexArgs` with `path`, `--force`, and `--dump-chunks`

## Outputs (must exist after completion)

- Working Phase 2 `vektor index --dump-chunks <file-or-dir>` inspection path
- Plain `vektor index <path>` Phase 2 path that updates hash state and reports changed/unchanged files
- Integration tests that isolate config and home-directory state

## Approach

- For file inputs, chunk only that file; for directory inputs, call discovery first and process discovered files in deterministic order.
- Keep `--dump-chunks` read-only with respect to embedding/vector stores.
- For plain index, compare hashes, chunk changed files, update HashStore statuses, and report a concise summary.
- Isolate tests from the developer's real `~/.vektor` directory.

## Acceptance criteria

- [ ] `vektor index --dump-chunks <file>` prints chunks and exits 0
- [ ] `vektor index --dump-chunks <dir>` discovers files and prints chunks deterministically
- [ ] Plain `vektor index <path>` updates HashStore state, reports changed/unchanged counts, and still avoids embedding/vector writes in Phase 2
- [ ] Running plain `vektor index <path>` twice on unchanged input reports zero changed files
- [ ] `vektor index --force <path>` bypasses stored file hashes and reprocesses discovered files without embedding/vector writes
- [ ] Output includes path, line range, language, symbol name/type, and full content hash
- [ ] Integration tests use isolated `HOME`/`USERPROFILE`

## Verification

```bash
cargo test dump_chunks
cargo test index_cli
cargo build
FAKE_HOME=$(mktemp -d)
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" ./target/debug/vektor index --dump-chunks src/
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" ./target/debug/vektor index src/
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" ./target/debug/vektor index src/
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" ./target/debug/vektor index --force src/
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Keep stdout user-facing for this one-shot command. MCP stdout cleanliness only applies to `vektor serve`.
