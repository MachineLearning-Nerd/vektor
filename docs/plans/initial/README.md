# Implementation Plan — Stage 2 (v0.1.0 → v0.4.x)

> Index of phases and tasks. Read [AGENTS.md](AGENTS.md) first for the execution contract.

This plan covers **Roadmap Stage 2** only (see [`VEKTOR_ROADMAP.md`](../../../VEKTOR_ROADMAP.md)). Stages 3–6 will get their own `docs/plans/stage-N/` directory written just before each stage begins.

---

## Current State

| Phase | Release tag | Status | Notes |
|---|---|---|---|
| Phase 0 — Scaffolding | (none — pre-`v0.1.0`) | ✅ **Done** (4 of 4) | Cargo init ✅ (`a3a2c2a`); hooks ✅ (`689d5b2`); deny ✅ (`08361cc`); CI ✅ (`73ed9d9` + `080305d`); GitHub repo at https://github.com/MachineLearning-Nerd/vektor (private). |
| Phase 1 — Skeleton | `v0.1.0` | ✅ **Done** — published (private) | Binary entry point, config, error types, CLI, tracing, and no-op MCP handlers implemented. `v0.1.0` tag + GitHub Release exist (private). Repo stays **private until launch (`v0.4.0`)** by decision; the public flip + unauthenticated-install gate are deferred to the launch milestone, not pending. |
| Phase 2 — Discovery + Chunking | `v0.2.0` | ✅ **Done** — published (private) | File discovery, HashStore, AST/sliding chunking, and Phase 2 `vektor index` behavior, merged to main via PR #1. `v0.2.0` tag + notes-only GitHub Release exist (private). |
| Phase 3 — Embedding + Storage | `v0.3.0` | ✅ **Done** — prepared (private, notes-only) | ONNX Jina v2 + OpenAI-compatible embedders + factory, LanceDB vector store (schema/cache/reindex/search/delete), content-addressed re-index cache, secret-aware indexing, `vektor models download`, real vector-only `index_codebase` MCP tool. `v0.3.0` version bumped + release notes drafted; tag/publish pending separate human authorization. |
| Phase 4 — Search + MCP | (interim) | ✅ **Done** — merged to main | Tantivy BM25 (`TextIndex`), RRF fusion + adaptive weights + synonym expansion, `search_hybrid` (`tokio::join!` + RRF), `index_codebase` extended to write both LanceDB + Tantivy, and real MCP dispatch for the 3 primary tools (`search_code` + naive `get_context_for_prompt`). Merged via PR #4 (`1b39f49`) + review-fix PRs #5/#6. Interim — no release tag (next tag `v0.4.0` after Phase 5). |
| Phase 5 — Context Assembly | `v0.4.0` | ⬜ Not started | TokenCounter, Deduplicator, RelatedExpander, QueryCache, ShallowIndexer, RecencyTracker. |
| Phase 6 — Launch Polish | `v0.4.0` | ⬜ Not started | `vektor init`, signed release pipeline, lightweight benchmark gate, `BENCHMARKS.md` baseline. |

**Active phase**: Phase 5 — Context Assembly (`v0.4.0`). Phases 0–4 are complete and merged to main. `v0.3.0` is prepared (private notes-only; tag/publish pending separate authorization); `v0.1.0` and `v0.2.0` are tagged + released (private); the public flip is deferred to launch (`v0.4.0`). Next: expand the [`phase-5-context-assembly/`](phase-5-context-assembly/README.md) task list into per-task files, then execute.

---

## Phase summaries

Each phase has a `phase-N/README.md` with the full task list. Phases 0, 1, 2, 3, and 4 have task-level files; phases 5–6 currently have task lists only and will be expanded before each phase begins.

### Phase 0 — Scaffolding
**Goal**: `git clone && cargo check` succeeds. No application code yet, but the build pipeline, CI, and pre-commit hooks all work.
**Tasks**: 4 ([`phase-0-scaffolding/README.md`](phase-0-scaffolding/README.md))

### Phase 1 — Skeleton (`v0.1.0`)
**Goal**: `vektor index` and `vektor serve` parse CLI args, `vektor serve` registers a no-op MCP server with rmcp 1.7. README and LICENSE in place. CI green on macOS + Linux.
**Tasks**: 8 after splitting 1.7 into release prep and publish ([`phase-1-skeleton/README.md`](phase-1-skeleton/README.md))

### Phase 2 — Discovery + Chunking (`v0.2.0`)
**Goal**: `vektor index <repo>` walks files (respecting `.gitignore`), tree-sitter chunks 5 languages with header preservation + sub-chunking, sliding-window fallback for other types. `--dump-chunks` shows what gets indexed.
**Tasks**: ~9 ([`phase-2-discovery-chunking/README.md`](phase-2-discovery-chunking/README.md))

### Phase 3 — Embedding + Storage (`v0.3.0`)
**Goal**: ONNX Jina v2 loads + embeds chunks, LanceDB stores them with proper Arrow schema. OpenAI-compatible backend is configurable; Ollama remains deferred. Secrets are skipped during indexing.
**Tasks**: 14 after splitting 3.7 into cache/reuse/insert subtasks ([`phase-3-embedding-storage/README.md`](phase-3-embedding-storage/README.md))

### Phase 4 — Search + MCP (interim)
**Goal**: Tantivy BM25 + dense vector search fuse via RRF with adaptive weights. MCP server dispatches `index_codebase`, `search_code`, `get_context_for_prompt` to handlers.
**Tasks**: 8 ([`phase-4-search-mcp/README.md`](phase-4-search-mcp/README.md))

### Phase 5 — Context Assembly (`v0.4.0`)
**Goal**: The differentiator. Token-budgeted context packages with deduplication, related-file expansion, query caching, shallow indexing, recency weighting, adaptive hybrid weights.
**Tasks**: ~12 ([`phase-5-context-assembly/README.md`](phase-5-context-assembly/README.md))

### Phase 6 — Launch Polish (`v0.4.0`)
**Goal**: `vektor init` writes MCP config for Claude Code/Cursor/Codex. Signed release pipeline on tag push. 20-query benchmark vs tokio prints to `BENCHMARKS.md`. Public `v0.4.0` release.
**Tasks**: ~5 ([`phase-6-launch-polish/README.md`](phase-6-launch-polish/README.md))

---

## How to use this plan

See [`AGENTS.md`](AGENTS.md) for the full execution contract. Quick summary:

1. Read AGENTS.md (one time per session)
2. Identify the active phase from the table above
3. Read that phase's README and pick the lowest-numbered unblocked task
4. Follow [DEPENDENCIES.md](DEPENDENCIES.md) to verify it's truly unblocked
5. Execute → verify → commit → mark ✅ Done
6. Repeat

---

## File index

```
docs/plans/initial/
├── AGENTS.md                           Execution contract (read first)
├── README.md                           This file — phase index + current state
├── DEPENDENCIES.md                     Cross-phase DAG + per-task dep table
├── phase-0-scaffolding/
│   ├── README.md                       Phase overview + task list
│   ├── 01-cargo-init.md                Task 0.1
│   ├── 02-ci-skeleton.md               Task 0.2
│   ├── 03-pre-commit-hooks.md          Task 0.3
│   └── 04-cargo-deny.md                Task 0.4
├── phase-1-skeleton/
│   ├── README.md
│   ├── 01-main-entrypoint.md           Task 1.1
│   ├── 02-error-module.md              Task 1.2
│   ├── 03-config-module.md             Task 1.3
│   ├── 04-cli-args.md                  Task 1.4
│   ├── 05-tracing-init.md              Task 1.5
│   ├── 06-mcp-noop-handlers.md         Task 1.6
│   ├── 07-v0.1.0-release.md            Task 1.7a release prep
│   └── 08-v0.1.0-publish.md            Task 1.7b release publish
├── phase-2-discovery-chunking/
│   ├── README.md
│   ├── 01-discover-files.md
│   ├── 02-hash-store.md
│   ├── 03-language-detection.md
│   ├── 04-parse-ast.md
│   ├── 05-ast-chunker.md
│   ├── 06-sliding-window-fallback.md
│   ├── 07-chunk-file-dispatcher.md
│   ├── 08-dump-chunks-cli.md
│   └── 09-v0.2.0-release.md
├── phase-3-embedding-storage/
│   ├── README.md
│   ├── 01-embedder-trait.md
│   ├── 02-onnx-embedder-new.md
│   ├── 03-onnx-embedder-embed.md
│   ├── 04-openai-compat-embedder.md
│   ├── 05-build-embedder-factory.md
│   ├── 06-vector-store-new.md
│   ├── 07a-vector-store-cache-read.md
│   ├── 07b-vector-store-reindex-reuse.md
│   ├── 07c-vector-store-insert-and-index-cli.md
│   ├── 08-vector-store-search.md
│   ├── 09-vector-store-delete-by-file.md
│   ├── 10-secret-detector.md
│   ├── 11-models-download.md
│   └── 12-v0.3.0-release.md
├── phase-4-search-mcp/
│   ├── README.md
│   ├── 01-text-index-new.md
│   ├── 02-text-index-add-chunks.md
│   ├── 03-text-index-search.md
│   ├── 04-rrf-fuse-adaptive-weights-synonyms.md
│   ├── 05-search-hybrid.md
│   ├── 06-index-codebase-orchestrator.md
│   ├── 07-mcp-server-bootstrap.md
│   └── 08-tool-handlers.md
├── phase-5-context-assembly/
│   └── README.md
└── phase-6-launch-polish/
    └── README.md
```

---

## Pre-planned vs just-in-time

Phases 0, 1, 2, 3, and 4 are **planned at the task level** and **complete** (Phases 2/3 published privately as `v0.2.0` / `v0.3.0`-prep; Phase 4 merged to main as an interim phase with no tag). Phase 5 is the active surface.

Phases 5–6 remain **planned at the phase level** with task lists only. Later per-task files are written **just before each phase begins** so that:

1. Lessons from earlier phases inform later tasks (we won't pre-write task 5.7 today based on assumptions we'll find wrong in Phase 4)
2. The plan stays a *tool*, not a stale document
3. Reviewers can validate the format on Phases 0/1 before we commit to per-task details for all 50+ tasks

The Phase 2 task files were expanded during `1.7a` release prep. The Phase 3 task files were expanded after the private `v0.2.0` publish gate. The Phase 4 task files were expanded by task 3.12 (Phase 3 closure). Phase 5 task files are being expanded now as the Phase 4 closure step (task 4.8's final deliverable).

---

*This is a living index. Update it as phases complete.*
