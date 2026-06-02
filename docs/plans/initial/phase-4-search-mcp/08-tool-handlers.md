# Task 4.8 — Tool handlers: `search_code` + basic `get_context_for_prompt`

**Phase**: 4 — Search + MCP
**Task ID**: 4.8
**PRD reference**: Section 9 (MCP Tools API — `search_code` / `get_context_for_prompt` request + response envelopes and `mode` options), Section 5.3 (in-memory `ContextPackage` / `ContextChunk` structs), Section 12 Week 4 Function 4.8
**Roadmap stage**: Stage 2 (interim — no release tag)
**Effort estimate**: M
**Depends on**: 4.5, 4.7
**Blocks**: 5.6

## Objective

Shape the real MCP responses for the two remaining stubbed tools. `search_code`
returns ranked hybrid results as PRD-shaped JSON. `get_context_for_prompt`
returns a **deliberately naive** `ContextPackage` (search → top-k → return, with
NO dedup, expansion, or budget allocation — those are Phase 5). The point is that
neither tool is a no-op after this task.

> Note: this Phase 4 `get_context_for_prompt` is intentionally naive — the full
> assembly pipeline (dedup, expansion, budget allocation) is Phase 5, and this
> same handler is upgraded at task 5.6. The executable dependency is `4.5, 4.7`
> (the phase README and DEPENDENCIES.md agree).

## Inputs (must exist before starting)

- `search_hybrid` (4.5) + the server state plumbing from 4.7.
- The `search_code` modes (`hybrid` | `semantic` | `keyword`) and request
  parameters (`query`, `path`, `top_k`, `mode`, `filter_ext`, `bypass_cache`) from
  PRD §9 (the JSON request/response envelopes and `mode` list live in §9, not §8).
- The `get_context_for_prompt` response envelope from PRD §9 (its `context` items
  use the wire field `relevance`), backed by the in-memory `ContextPackage` /
  `ContextChunk` structs in PRD §5.3 (which use `relevance_score` / `rel_path`),
  plus `index_status` (PRD §4.4) / `index_coverage_pct` (PRD §4.5) — §9 enumerates
  both in every response envelope.

## Outputs (must exist after completion)

- `handle_search_code(args)` parses `path`, `query`, optional `mode` (default
  `hybrid`), optional `top_k`, optional `filter_ext`, and `bypass_cache`, runs
  `search_hybrid`, and returns the PRD §9 response envelope:
  `results: [{ file, lines, symbol, type, language, score, reason, snippet }]`
  plus `metadata: { search_time_ms, mode, cache_hit, index_status,
  index_coverage_pct, result_confidence, missing_context_warnings }`. An
  additive `chunk_id` field is acceptable for cross-store debugging, but do not
  replace the PRD-facing field names with internal names. Errors → JSON error
  object (mirror `handle_index_codebase`'s pattern), never panic, never stdout.
  `filter_ext` limits returned results to matching file suffixes (apply it in the
  retrievers where practical and post-filter the merged results as a backstop).
  Normalize filters the same way as `index_codebase.extensions`: lowercase, no
  leading dot, and reject empty/non-string entries as JSON argument errors.
  `bypass_cache` is accepted for schema compatibility; Phase 4 has no query cache,
  so responses should report `cache_hit: false`.
- `handle_get_context_for_prompt(args)` parses `query`, `path`, and the optional
  §9 fields (`token_budget`, `max_files`, `include_related`, `min_relevance`,
  `include_docs`, `bypass_cache`, `scope`), runs `search_hybrid`, and returns a
  `ContextPackage`-shaped JSON matching the PRD §9 `get_context_for_prompt`
  response envelope (backed by the §5.3 struct):
  `context: [{ file, lines, symbol, type, language, relevance, source, reason,
  content }]` plus `metadata` keys for files/chunks returned, budget placeholders,
  `search_time_ms`, `cache_hit`, `index_status`, `index_coverage_pct`,
  `result_confidence`, `budget_gap_reason`, `missing_context_warnings`,
  `suggested_action`, and `clusters`. Phase 4 may use naive placeholder values
  for budget/confidence fields, but the shape must be stable for Phase 5.

## Approach

- Mirror the existing `run_index_tool` arg-parsing/error-mapping pattern in
  `src/mcp/handlers.rs` (missing/typed args → JSON error string; success → shaped
  JSON). Keep handlers thin: parse → call engine → shape JSON.
- For `search_code`, map `HybridResult` → the PRD §9 result object; pass `mode`
  through to `search_hybrid`, convert `rel_path` to `file`, `start_line/end_line`
  to the `"start-end"` `lines` string, `symbol_name` to `symbol`, and
  `relevance_score` to `score`. Validate `mode`, `top_k`, and `filter_ext`
  types. Treat `bypass_cache` as accepted/no-op until QueryCache lands in Phase 5.
- For `get_context_for_prompt`, build the minimal `ContextPackage`: take top-k
  hybrid results, emit them as the package's `context` list — converting the
  internal `relevance_score` to the §9 wire field `relevance` (the same way
  `search_code` maps it to `score`) — set `index_status` from whether a full index
  exists, and a naive
  `result_confidence` (e.g. High if top score above a threshold). Fill budget
  metadata with honest placeholders (`total_tokens` can be an estimate,
  `chunks_deduplicated = 0`, `budget_gap_reason = null` unless a PRD-defined
  reason applies).
  Honor the easy wire-level controls in this naive implementation:
  `max_files` caps distinct files, `min_relevance` filters low-score chunks,
  `include_docs=false` filters obvious doc-only paths/extensions, and `scope`
  constrains results to that workspace-relative prefix when present. Accept
  `token_budget`, `include_related`, and `bypass_cache`, but do not implement
  budget allocation, related expansion, or query caching here; surface honest
  metadata/warnings when those Phase 5 behaviors are deferred.
  Explicitly do NOT implement TokenCounter/Deduplicator/RelatedExpander/budget —
  those are Phase 5 (tasks 5.1–5.5) and upgrade this handler at task 5.6.
- Keep both handlers stdout-silent; log via tracing.

## Acceptance criteria

- [ ] `search_code` returns real ranked results (not the "not implemented" stub)
      and honors `mode` (hybrid/semantic/keyword) in the PRD §9
      `{ results, metadata }` envelope.
- [ ] `search_code.filter_ext` limits results to matching suffixes; `bypass_cache`
      is accepted and reports `cache_hit: false` until QueryCache exists.
- [ ] `filter_ext` accepts PRD-style dotted values and bare values
      case-insensitively, and rejects empty/non-string entries as JSON argument
      errors.
- [ ] `get_context_for_prompt` returns a structured `ContextPackage` JSON matching
      the PRD §9 `{ context, metadata }` envelope (backed by the §5.3 struct, even
      with naive budget/confidence) — not a stub.
- [ ] `get_context_for_prompt` honors `max_files`, `min_relevance`,
      `include_docs=false`, and `scope` at the simple search-result shaping layer;
      it accepts `token_budget`, `include_related`, and `bypass_cache` without
      pretending Phase 5 budget/related/cache behavior exists.
- [ ] Both handlers return a JSON error object on bad/missing args; neither panics
      nor writes to stdout.
- [ ] `index_status` present in both responses (PRD §4.4; delivered via the §9
      response metadata).
- [ ] No dedup/expansion/budget logic is added here (kept for Phase 5).
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test mcp::handlers::tests::search_code_returns_ranked_results
cargo test mcp::handlers::tests::search_code_honors_filter_ext_and_bypass_cache
cargo test mcp::handlers::tests::search_code_normalizes_filter_ext
cargo test mcp::handlers::tests::get_context_returns_context_package
cargo test mcp::handlers::tests::get_context_honors_phase4_wire_controls
cargo test mcp::handlers::tests::handlers_reject_bad_args_as_json_error
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- `get_context_for_prompt` is intentionally naive at Phase 4 (phase README): the
  handler exists so the tool isn't a no-op; the QUALITY (dedup, expansion,
  token-budgeted assembly) lands in Phase 5. Do not gold-plate it here.
- Use the fake-embedder test seam (as Phase 3's `index_codebase` test does) so
  handler tests run without a downloaded model.
- This task closes Phase 4: on completion, run the AGENTS.md phase-completion
  checklist (mark all 8 tasks ✅ with hashes; update README/DEPENDENCIES/ROADMAP
  current-state; expand `phase-5-context-assembly/` per-task files). No release
  tag — the next tag is `v0.4.0` after Phase 5.
