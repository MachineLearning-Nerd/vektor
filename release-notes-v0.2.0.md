# Vektor v0.2.0 Release Notes Draft

> Draft only. Do not publish, tag, or create a GitHub Release until explicitly approved.

## Summary

`v0.2.0` turns `vektor index` from a Phase 1 stub into a local discovery and chunking pipeline. It can walk a project, respect ignore files, chunk supported source files with tree-sitter, fall back to sliding-window chunks for other readable files, and persist file-hash state for incremental re-runs.

This release still does not embed chunks, write LanceDB vectors, build BM25 indexes, or enable real MCP search/context handlers.

## What Changed

- Added deterministic file discovery with nested `.gitignore`, `.git/info/exclude`, generated/cache directory skips, and max-file-size enforcement.
- Added a SQLite-backed `HashStore` under the configured Vektor data directory, scoped by project root.
- Added shared chunk metadata, full content hashes, stable chunk IDs, and language detection for Python, TypeScript, TSX, JavaScript, JSX, Rust, and Go.
- Added tree-sitter parser setup and AST semantic chunk extraction for the supported languages.
- Added sliding-window fallback chunking for unsupported files, parser setup failures, syntax-error trees, and docs/text files.
- Implemented Phase 2 `vektor index` behavior:
  - `vektor index --dump-chunks <file-or-dir>` prints deterministic chunk metadata/content and exits without creating state.
  - `vektor index <path>` discovers files, hashes changed files, chunks changed files, updates HashStore state, and reports changed/unchanged/failed/chunk counts.
  - `vektor index --force <path>` bypasses stored hashes and reprocesses files.

## Verification Before Publish

Run the final acceptance gate before publishing:

```bash
cargo fmt --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked

FAKE_HOME=$(mktemp -d)
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" ./target/debug/vektor index --dump-chunks src/
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" ./target/debug/vektor index src/
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" ./target/debug/vektor index src/
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" ./target/debug/vektor index --force src/

git diff --check
```

## Deferred

- Embeddings and model download UX remain Phase 3.
- LanceDB vector storage remains Phase 3.
- Secret-aware indexing remains Phase 3.
- BM25, hybrid search, and real MCP handlers remain later phases.
- Binary release artifacts remain deferred to v0.4.0 / task 6.2.
