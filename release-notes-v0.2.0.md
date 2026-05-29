# Vektor v0.2.0 — Discovery + Chunking

## Summary

`v0.2.0` turns `vektor index` from a Phase 1 stub into a local discovery and chunking pipeline. It walks a project, respects ignore files, chunks supported source files with tree-sitter, falls back to sliding-window chunks for other readable files, and persists file-hash state for incremental re-runs.

This release still does not embed chunks, write LanceDB vectors, build BM25 indexes, or enable real MCP search/context handlers — those land in later phases.

## What's new

- Deterministic file discovery honoring nested `.gitignore`, `.git/info/exclude`, generated/cache-directory skips, and max-file-size enforcement.
- A SQLite-backed `HashStore` under the configured Vektor data directory, scoped by project root, for incremental re-indexing.
- Shared chunk metadata, full content hashes, stable content-addressed chunk IDs, and language detection for Python, TypeScript, TSX, JavaScript, JSX, Rust, and Go.
- Tree-sitter parser setup and AST-based semantic chunk extraction for the supported languages, with header preservation and sub-chunking of oversized functions.
- Sliding-window fallback chunking for unsupported files, parser-setup failures, syntax-error trees, and docs/text files.
- Phase 2 `vektor index` behavior:
  - `vektor index --dump-chunks <file-or-dir>` prints deterministic chunk metadata/content and exits without writing state.
  - `vektor index <path>` discovers files, hashes changed files, chunks them, updates the HashStore, and reports changed/unchanged/failed/chunk counts.
  - `vektor index --force <path>` bypasses stored hashes and reprocesses everything.

## Install

Prerequisites: Rust 1.91+ and `protoc` (macOS: `brew install protobuf`; Linux: `apt-get install protobuf-compiler` or `dnf install protobuf-compiler`).

```
cargo install --git https://github.com/MachineLearning-Nerd/vektor --tag v0.2.0 --locked
```

## Deferred to later phases

- Embeddings and model-download UX — Phase 3.
- LanceDB vector storage — Phase 3.
- Secret-aware indexing — Phase 3.
- BM25, hybrid search, and real MCP handlers — later phases.
- Prebuilt binary release artifacts — `v0.4.0` / task 6.2.
