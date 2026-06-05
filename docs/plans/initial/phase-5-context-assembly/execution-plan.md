# Phase 5 Execution Plan — Context Assembly (`v0.4.0`)

## Operating Model

For every task:
1. Implement per-task spec from `docs/plans/initial/phase-5-context-assembly/XX-*.md`.
2. Run task-specific verification commands from that task file.
3. Review implementation (tests + behavior checks + existing style + comments).
4. Implement any review comments and re-run impacted verification.
5. Mark task status in this file, then move to the next task.

Guideline: **do not move forward** until task acceptance checks are green and comments are closed.

## Ground Rules for this phase

- Keep changes scoped to task-local files and avoid changing MCP schemas/CLI args.
- No new dependencies unless explicitly required in the task.
- Keep the handler and wire contract stable (`get_context_for_prompt` response shape unchanged).
- Follow existing project patterns from Phase 4/3/2 code.
- Use cache and tracker state updates to avoid cross-project leakage.
- Treat all plan status updates as manual checkpoints (not an automated state machine).

## Task Order and Review Gates

| Order | Task | File | Depends on | Implementation Gate | Status |
|---|---|---|---|---|---|
| 1 | 5.1 `TokenCounter::estimate` | `01-token-counter.md` | 4.5 | Language-specific heuristic + cached `tiktoken-rs` exact count implemented; task tests pass. | [x] Verified with `cargo test context::budget`, `cargo build`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`. |
| 2 | 5.2 `Deduplicator::deduplicate` | `02-deduplicator.md` | 4.5 | Overlap rule (>50% of smaller chunk) correct and deterministic tests pass. | [x] Verified with `cargo test context::dedup`, `cargo build`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`. |
| 3 | 5.3 `RelatedExpander::expand` | `03-related-expander.md` | 4.5 | Tiered chunk-level expansion logic + caps + hub skip + chunk gating pass. | [x] Verified with `cargo test context::expander`, `cargo build`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`. Live tiers are test-file and sibling-barrel; import-graph tiers use the new store seam until dependency data exists. |
| 4 | 5.4 `QueryCache` | `04-query-cache.md` | 4.5 | LRU + TTL + file-level invalidation + bypass-cache path validated. | [x] Verified with `cargo test context::cache`, `cargo build`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`. |
| 5 | 5.9 `RecencyTracker::score` | `09-recency-tracker.md` | 4.5 | 24h/7d decay + >0.3 gate + deterministic now injection tested. | [x] Verified with `cargo test context::recency`, `cargo build`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`. |
| 6 | 5.5 `ContextAssembler::assemble` | `05-context-assembler.md` | 5.1, 5.2, 5.3, 5.4, 5.9 | Full PRD §5.4 pipeline implemented and package shaping complete. | [x] Verified with `cargo test context`, `cargo build`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`. |
| 7 | 5.6 `handle_get_context_for_prompt` | `06-get-context-handler.md` | 4.8, 5.5 | Handler delegates to assembler and emits real metadata/confidence/budget fields. | [x] Verified with `cargo test mcp::handlers`, `cargo test mcp::server`, `cargo build`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`. |
| 8 | 5.7 `ShallowIndexer::build` | `07-shallow-indexer.md` | 2.6, 4.1 | Done. Added shallow BM25 docs from path/head/tail/declarations, deterministic shallow IDs, `index_depth = "shallow"`, and deep delete handoff. Verified with `cargo test shallow`, `cargo test text_index`, `cargo build`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`. Local review clean; orchestration remains deferred to 5.8 per spec. | [x] |
| 9 | 5.8 `IndexStatusTracker` | `08-index-status-tracker.md` | 5.7 | Done. Added per-project `IndexStatusTracker`, disk seeding from searchable text index + full vector metadata, shallow/partial and full transitions in MCP index flow, building short-circuit without embedder load, partial shallow keyword routing for search/context, and persisted full-index metadata. Verified with `cargo test index_status`, `cargo test mcp::handlers`, `cargo test mcp::handlers::tests::keyword_search_does_not_build_embedder`, `cargo test text_index`, `cargo test search`, `cargo build`, `cargo fmt --check`, and `cargo clippy --workspace --all-targets -- -D warnings`. Local review clean. | [x] |
| 10 | 5.12 `WarmUp::run` | `12-warmup.md` | 3.2 | Done. Added backend-agnostic warm-up through `embed_documents` for batch sizes `1` and `32`, eager `vektor serve` startup warm-up using the shared embedder cache, fail-fast error propagation, and server/unit coverage. The stdio smoke uses an explicit warm-up skip because it has no model artifacts. Verified with `cargo test warmup`, `cargo test mcp::server`, `cargo test --test mcp_serve`, `cargo build`, `cargo fmt --check`, and `cargo clippy --workspace --all-targets -- -D warnings`. Local review clean. | [x] |
| 11 | 5.13 Stage-5 integration | `13-stage5-integration.md` | 5.6, 5.8, 5.12 | Done. Added `tests/stage5_context.rs`, an MCP-process integration using a local OpenAI-compatible mock server to verify serve warm-up, indexing, and `get_context_for_prompt` full `ContextPackage` wire shape with `index_status = "full"` and token budget not exceeded. Component exit criteria remain covered by task-level tests (`context`, `mcp::handlers`, `search`); real-model warm-query `<150ms` remains model-gated/manual per the task notes. Verified with `cargo build`, `cargo test --test stage5_context`, `cargo test context`, `cargo test mcp::handlers`, `cargo test search`, `cargo fmt --check`, and `cargo clippy --workspace --all-targets -- -D warnings`. Local review clean. | [x] |

## Completion Gate Before Next Phase

- [x] All 11 actionable tasks above are marked done.
- [x] Phase exit criteria in `README.md` are verified and updated in this execution tracker.
- [x] `ContextPackage` fields are fully present with stable metadata:
  - `chunks`, `files_included`, `total_tokens`, `budget_used_pct`,
  - `missing_context_warnings`, `result_confidence`, `budget_gap_reason`,
  - `clusters`.
- [x] The phase-level task table in `README.md` is updated to reflect final status.
- [x] Next phase prep is started only after closing unresolved review comments.

## Review + Comment Workflow (always run for every task)

- Open question checks:
  - Are tests targeted to task acceptance criteria passing?
  - Are no new cross-task regressions introduced?
  - Is any task-specific contract (handler shape, index status string, wire field mapping) still stable?
- Comment pass:
  - Address cleanup items, unsafe unwraps, and unresolved review comments introduced in the task.
  - Re-run task verification after every comment-driven fix.
  - If comments remain open, keep task status as `In Review` and do not start the next task.
- Plan update:
  - Mark each task as done immediately after review closure.
  - Add one-line note under that task about verification command executed.

## Suggested sprint loop (short)

1. Pick task by table order above.
2. Implement one task fully.
3. Run at least one targeted test command from that task.
4. Run one cross-task guard command (`cargo test` subset relevant to context/search).
5. Review and fix comments.
6. Update task status checkbox here.
7. Repeat for next task.

When any task is blocked, pause and record the blocker before starting unrelated work.
