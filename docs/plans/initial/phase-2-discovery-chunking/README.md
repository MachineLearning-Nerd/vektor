# Phase 2 — Discovery + Chunking (`v0.2.0`)

> **Goal**: `vektor index <repo>` walks the filesystem respecting `.gitignore`, chunks source files via tree-sitter for 5 languages (with header preservation + sub-chunking), and falls back to a sliding-window chunker for other types. `vektor index --dump-chunks <file>` prints chunks for inspection. No embedding or vector storage yet; the only persisted state is the SQLite HashStore.

**Roadmap mapping**: Stage 2 / `v0.2.0`
**PRD mapping**: Section 4.2 (Chunker), Section 12 Week 2 Functions 2.1–2.5, plus `hash_file`/HashStore from Functions 1.3–1.4
**Effort estimate**: 3–5 weeks of focused part-time work
**Status**: ✅ Done — merged to main via PR #1; `v0.2.0` tag + notes-only GitHub Release published (private). Public flip deferred to launch (`v0.4.0`).

---

## Task list

Implementation tasks 2.1-2.8 are complete and verified locally. Task 2.9 is limited to release-readiness/tracker docs in this branch; irreversible publish actions are deliberately deferred.

| ID | Task | Effort | Depends on | Status | Commit |
|---|---|---|---|---|---|
| 2.1 | [`discover_files(root, config) -> Vec<PathBuf>`](01-discover-files.md) — `ignore` crate, respect `.gitignore` + skip lists | M | 1.7b | ✅ | `e8fb34c` |
| 2.2 | [`HashStore` (state.db SQLite)](02-hash-store.md) — file hashes, full chunk content hashes, get/set/is_changed/get_pending per Functions 1.3–1.4 | M | 2.1 | ✅ | `7e6b004` |
| 2.3 | [`Chunk` + `detect_language(path) -> Option<Language>`](03-language-detection.md) — shared chunk metadata and extension-to-language enum | S | 1.7b | ✅ | `24bb354` |
| 2.4 | [`parse_ast(content, language) -> Tree`](04-parse-ast.md) — tree-sitter parser construction | S | 2.3 | ✅ | `e171af2` |
| 2.5 | [`extract_chunks_ast`](05-ast-chunker.md) for Python, TypeScript, JavaScript, Rust, Go — with header preservation + sub-chunking at 200 lines | L | 2.4 | ✅ | `d0a58f7` |
| 2.6 | [`extract_chunks_sliding`](06-sliding-window-fallback.md) — 80-line window 25% overlap for code; 40-line 40% overlap for `.md`/`.txt`/`.rst` | M | 2.3 | ✅ | `8a5655b` |
| 2.7 | [`chunk_file(path, content, config) -> Vec<Chunk>`](07-chunk-file-dispatcher.md) — top-level dispatcher with content-addressed chunk IDs | M | 2.2, 2.5, 2.6 | ✅ | `ac3f9f8` |
| 2.8 | [`vektor index` Phase 2 behavior](08-dump-chunks-cli.md) — plain index updates HashStore; `--dump-chunks` pretty-prints chunks | M | 2.2, 2.7 | ✅ | `aca571b` |
| 2.9 | [`v0.2.0` release readiness + Phase 3 handoff docs](09-v0.2.0-release.md) | M | 2.8 | ✅ | this change |

---

## Phase exit criteria

Implementation gates:

- [x] Tasks 2.1-2.8 marked ✅ Done with implementation commit hashes
- [x] `vektor index --dump-chunks src/` on Vektor's own source prints non-trivial AST chunks (function bodies, struct defs, etc.)
- [x] Header preservation verified: an oversized test fixture produces sub-chunks with the parent signature prepended after the first chunk
- [x] Sub-chunk size cap: no chunk exceeds 200 configured source lines in tests
- [x] Content-addressed chunk IDs use full chunk content hashes; chunking the same file twice produces identical IDs (test via `cargo test`)
- [x] `.gitignore` honored: deliberately ignored files are not returned by `discover_files`
- [x] HashStore persists to a project-scoped `state.db` under `config.index.data_dir`; running `vektor index` twice on unchanged input reports zero changed files

Publish gates:

- [ ] Final release commit pushed to `origin/main`
- [ ] CI matrix green on the exact release commit
- [ ] Git tag `v0.2.0` pushed
- [ ] GitHub Release notes published. Binary release artifacts are still deferred to v0.4.0 / task 6.2.
- [ ] Phase 3 per-task files written before Phase 3 implementation starts

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
- **Hash store crash recovery**: if indexing stops mid-run, restarting must resume from `pending` rows. Unit tests can seed pending rows directly; only add OS-level kill fixtures if the implementation needs that coverage.

---

## When this phase completes

1. Keep tasks 2.1-2.8 marked ✅ in the table with commit hashes.
2. Keep `v0.2.0` tag/GitHub Release pending until explicit publish approval.
3. Update Current State tables (this README, `docs/plans/initial/README.md`, `VEKTOR_ROADMAP.md`).
4. Treat Phase 3 as readiness-only until its per-task files are expanded and the release/publish gate is approved.
