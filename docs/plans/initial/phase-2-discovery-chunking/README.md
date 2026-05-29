# Phase 2 — Discovery + Chunking (`v0.2.0`)

> **Goal**: `vektor index <repo>` walks the filesystem respecting `.gitignore`, chunks source files via tree-sitter for 5 languages (with header preservation + sub-chunking), and falls back to a sliding-window chunker for other types. `vektor index --dump-chunks <file>` prints chunks for inspection. No embedding or storage yet.

**Roadmap mapping**: Stage 2 / `v0.2.0`
**PRD mapping**: Section 4.2 (Chunker), Section 12 Week 2 Functions 2.1–2.5, plus a HashStore from Function 1.4
**Effort estimate**: 3–5 weeks of focused part-time work
**Status**: ⬜ Not started — per-task files written in 1.7a release prep; Phase 2 remains blocked until 1.7b publishes `v0.1.0`

---

## Task list

> Per-task `.md` files now exist. Do not start Phase 2 implementation until `v0.1.0` is published by task 1.7b.

| ID | Task | Effort | Depends on | Status |
|---|---|---|---|---|
| 2.1 | [`discover_files(root, config) -> Vec<PathBuf>`](01-discover-files.md) — `ignore` crate, respect `.gitignore` + skip lists | M | 1.7b | ⬜ |
| 2.2 | [`HashStore` (state.db SQLite)](02-hash-store.md) — get/set/is_changed/get_pending per Function 1.4 | M | 2.1 | ⬜ |
| 2.3 | [`detect_language(path) -> Option<Language>`](03-language-detection.md) — extension to tree-sitter Language enum | S | 1.7b | ⬜ |
| 2.4 | [`parse_ast(content, language) -> Tree`](04-parse-ast.md) — tree-sitter parser construction | S | 2.3 | ⬜ |
| 2.5 | [`extract_chunks_ast`](05-ast-chunker.md) for Python, TypeScript, JavaScript, Rust, Go — with header preservation + sub-chunking at 200 lines | L | 2.4 | ⬜ |
| 2.6 | [`extract_chunks_sliding`](06-sliding-window-fallback.md) — 80-line window 25% overlap for code; 40-line 40% overlap for `.md`/`.txt`/`.rst` | M | 1.7b | ⬜ |
| 2.7 | [`chunk_file(path, content) -> Vec<Chunk>`](07-chunk-file-dispatcher.md) — top-level dispatcher with content-addressed chunk IDs | M | 2.5, 2.6 | ⬜ |
| 2.8 | [`vektor index --dump-chunks <path>`](08-dump-chunks-cli.md) CLI flag — chunks one file or directory and pretty-prints results | M | 2.2, 2.7 | ⬜ |
| 2.9 | [`v0.2.0` release tag + Phase 3 expansion](09-v0.2.0-release.md) | M | 2.8 | ⬜ |

---

## Phase exit criteria

All must be true before tagging `v0.2.0`:

- [ ] All 9 tasks above marked ✅ Done
- [ ] `vektor index --dump-chunks src/` on Vektor's own source prints non-trivial AST chunks (function bodies, struct defs, etc.)
- [ ] Header preservation verified: a 250-line function in a test fixture produces 2 sub-chunks, the second prepending the parent signature
- [ ] Sub-chunk size cap: no chunk exceeds 200 source lines
- [ ] Content-addressed chunk IDs: chunking the same file twice produces identical IDs (test via `cargo test`)
- [ ] `.gitignore` honored: a deliberately-ignored file (added to `.gitignore` in the test repo) is not in the output of `discover_files`
- [ ] HashStore persists to `~/.vektor/<project>/state.db`; running `vektor index` twice on an unchanged repo produces "0 files changed" output
- [ ] CI matrix green; tag pushed; GitHub Release notes published. Binary release artifacts are still deferred to v0.4.0 / task 6.2.
- [ ] Per-task files for Phase 3 written before phase 3 work starts (task 2.9 closes the expansion)

---

## What v0.2.0 *intentionally does not* include

- Embedding or vector storage (Phase 3)
- BM25 indexing (Phase 4)
- MCP tool handlers that actually do something (Phase 4)
- Real-time file watching (Stage 4 / v1.0)

The MCP server still responds with no-op handlers from v0.1.0. `vektor index` actually does chunking work now, but it doesn't write to any backing store beyond the HashStore SQLite. Think of v0.2.0 as "we can split code into pieces, and we know which pieces have changed since last time."

---

## Notes

- **Tree-sitter grammar pitfalls**: each language grammar has slightly different node-name conventions. `function_declaration` vs `function_definition` vs `function_item`. PRD Section 12 Function 2.3 lists the per-language node types — verify each against the current tree-sitter grammar version before implementing.
- **Sub-chunk header preservation** (PRD v2.3 fix): always prepend the parent node's signature (first 3-5 lines) to each sub-chunk. Tests must cover this.
- **`.gitignore` semantics**: the `ignore` crate handles nested `.gitignore` files, `.git/info/exclude`, global gitignore, etc. Don't roll your own.
- **Max file size**: respect `config.index.max_file_size_kb` (default 512KB per PRD Section 6.3). Files larger than this are skipped with a debug-level log message.
- **Hash store crash recovery**: if a process is killed mid-index, restarting must resume from the `pending` files. Test this with a SIGKILL-and-restart fixture.

---

## When this phase completes

1. Mark all tasks ✅ in the table with commit hashes
2. Tag `v0.2.0`
3. Update Current State tables (this README, `docs/plans/initial/README.md`, `VEKTOR_ROADMAP.md`)
4. Expand `phase-3-embedding-storage/` from task list to per-task files (task 2.9 closes this)
