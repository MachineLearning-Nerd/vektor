# Implementation Plan — Stage 2 (v0.1.0 → v0.4.x)

> Index of phases and tasks. Read [AGENTS.md](AGENTS.md) first for the execution contract.

This plan covers **Roadmap Stage 2** only (see [`VEKTOR_ROADMAP.md`](../../../VEKTOR_ROADMAP.md)). Stages 3–6 will get their own `docs/plans/stage-N/` directory written just before each stage begins.

---

## Current State

| Phase | Release tag | Status | Notes |
|---|---|---|---|
| Phase 0 — Scaffolding | (none — pre-`v0.1.0`) | ✅ **Done** (4 of 4) | Cargo init ✅ (`a3a2c2a`); hooks ✅ (`689d5b2`); deny ✅ (`08361cc`); CI ✅ (`73ed9d9` + `080305d`); GitHub repo at https://github.com/MachineLearning-Nerd/vektor (private). |
| Phase 1 — Skeleton | `v0.1.0` | ⬜ Not started | Binary entry point, config, error types, no-op MCP handlers, README/LICENSE/tracing wired. |
| Phase 2 — Discovery + Chunking | `v0.2.0` | ⬜ Not started | File walking, hash store, AST chunker, sliding fallback. |
| Phase 3 — Embedding + Storage | `v0.3.0` | ⬜ Not started | ONNX/OpenAI/Ollama backends, LanceDB store, secret-aware indexing (B1.2/B1.5). |
| Phase 4 — Search + MCP | (interim) | ⬜ Not started | Tantivy BM25, RRF fusion, MCP tool dispatch for 3 primary tools. |
| Phase 5 — Context Assembly | `v0.4.0` | ⬜ Not started | TokenCounter, Deduplicator, RelatedExpander, QueryCache, ShallowIndexer, RecencyTracker. |
| Phase 6 — Launch Polish | `v0.4.0` | ⬜ Not started | `vektor init`, signed release pipeline, lightweight benchmark gate, `BENCHMARKS.md` baseline. |

**Active phase**: Phase 1 (Phase 0 complete; task 1.1 is the next unblocked work).

---

## Phase summaries

Each phase has a `phase-N/README.md` with the full task list. Phases 0 and 1 have fully-detailed per-task files; phases 2–6 currently have task lists only and will be expanded before each phase begins.

### Phase 0 — Scaffolding
**Goal**: `git clone && cargo check` succeeds. No application code yet, but the build pipeline, CI, and pre-commit hooks all work.
**Tasks**: 4 ([`phase-0-scaffolding/README.md`](phase-0-scaffolding/README.md))

### Phase 1 — Skeleton (`v0.1.0`)
**Goal**: `vektor index` and `vektor serve` parse CLI args, `vektor serve` registers a no-op MCP server with rmcp 1.7. README and LICENSE in place. CI green on macOS + Linux.
**Tasks**: 7 ([`phase-1-skeleton/README.md`](phase-1-skeleton/README.md))

### Phase 2 — Discovery + Chunking (`v0.2.0`)
**Goal**: `vektor index <repo>` walks files (respecting `.gitignore`), tree-sitter chunks 5 languages with header preservation + sub-chunking, sliding-window fallback for other types. `--dump-chunks` shows what gets indexed.
**Tasks**: ~9 ([`phase-2-discovery-chunking/README.md`](phase-2-discovery-chunking/README.md))

### Phase 3 — Embedding + Storage (`v0.3.0`)
**Goal**: ONNX Jina v2 loads + embeds chunks, LanceDB stores them with proper Arrow schema. OpenAI/Ollama backends configurable. Secrets are skipped during indexing.
**Tasks**: ~11 ([`phase-3-embedding-storage/README.md`](phase-3-embedding-storage/README.md))

### Phase 4 — Search + MCP (interim)
**Goal**: Tantivy BM25 + dense vector search fuse via RRF with adaptive weights. MCP server dispatches `index_codebase`, `search_code`, `get_context_for_prompt` to handlers.
**Tasks**: ~8 ([`phase-4-search-mcp/README.md`](phase-4-search-mcp/README.md))

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
│   └── 07-v0.1.0-release.md            Task 1.7
├── phase-2-discovery-chunking/
│   └── README.md                       Task list only — per-task files written before Phase 2 begins
├── phase-3-embedding-storage/
│   └── README.md
├── phase-4-search-mcp/
│   └── README.md
├── phase-5-context-assembly/
│   └── README.md
└── phase-6-launch-polish/
    └── README.md
```

---

## Pre-planned vs just-in-time

Phases 0 and 1 are **pre-planned at the task level** because they happen next and the work is well-understood.

Phases 2–6 are **planned at the phase level** with task lists only. Per-task files are written **just before each phase begins** so that:

1. Lessons from earlier phases inform later tasks (we won't pre-write task 5.7 today based on assumptions we'll find wrong in Phase 3)
2. The plan stays a *tool*, not a stale document
3. Reviewers can validate the format on Phases 0/1 before we commit to per-task details for all 50+ tasks

When Phase 2 begins (after Phase 1 ships `v0.1.0`), the first action is "expand `phase-2-discovery-chunking/` from task list to per-task files." That expansion is itself a task — see Phase 1's task `1.7` for the trigger.

---

*This is a living index. Update it as phases complete.*
