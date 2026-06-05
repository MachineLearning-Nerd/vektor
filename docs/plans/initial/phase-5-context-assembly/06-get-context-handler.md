# Task 5.6 — `handle_get_context_for_prompt` full assembly pipeline

**Phase**: 5 — Context Assembly
**Task ID**: 5.6
**PRD reference**: Section 9 (MCP Tools API — `get_context_for_prompt` request + response envelope), Section 5.3 (ContextAssembler / ContextPackage)
**Roadmap stage**: Stage 2 / v0.4.0
**Effort estimate**: M
**Depends on**: 4.8, 5.5
**Blocks**: 5.13

## Objective

Replace the **deliberately naive** Phase 4 `get_context_for_prompt` body (task 4.8) with
the full Phase 5 assembly pipeline. The handler now parses the §9 args, consults the
`QueryCache` (5.4), runs `search_hybrid` (4.5), feeds results through
`ContextAssembler::assemble` (5.5), and emits the same PRD §9 `{ context, metadata }`
envelope — but the dedup, related expansion, recency, and token-budget numbers are now
**real**, not the Phase 4 stubs. The MCP tool schema does NOT change (no regression).

> Note: this is a body swap, not a new tool. The Phase 4 handler at
> `src/mcp/handlers.rs` (`run_context_with_embedder_cached` →
> `context_results_to_json`) already shapes the §9 envelope naively; this task points it
> at the assembler and removes the "deferred to Phase 5" warnings that no longer apply.

## Inputs (must exist before starting)

- The existing Phase 4 handler plumbing in `src/mcp/handlers.rs`:
  `handle_get_context_for_prompt[_with_config|_with_caches]`, `run_context_tool_with_config`,
  `parse_context_tool_request` → `ContextToolRequest`, the `EmbedderCache` /
  `IndexHealthCache` seams, and `search_project` (which wraps `search_hybrid`).
- `ContextAssembler::assemble(results, &AssemblyConfig) -> Result<ContextPackage>` (5.5)
  plus the PRD §5.3 structs (`ContextPackage`, `ContextChunk`, `Confidence`, `GapReason`,
  `ResultCluster`).
- `QueryCache` (5.4) — LRU keyed on `(query_text, search_mode, project_hash)`, 60s TTL,
  file-level invalidation, honoring `bypass_cache`.
- The §9 request fields: `query`, `path`, `token_budget`, `max_files`, `include_related`,
  `min_relevance`, `include_docs`, `bypass_cache`, `scope` (already parsed by
  `parse_context_tool_request`).
- The §9 response envelope: `{ context: [{ file, lines, symbol, type, language, relevance,
  source, reason, content }], metadata: {...} }`.

## Outputs (must exist after completion)

- `handle_get_context_for_prompt` (and its `_with_config` / `_with_caches` variants)
  return the §9 envelope backed by the real `ContextPackage`, with the same signatures
  as Phase 4 — only the body of `run_context_with_embedder_cached` changes.
- The pipeline inside the handler:
  1. `parse_context_tool_request` → `ContextToolRequest` (unchanged).
  2. Build `AssemblyConfig` from the request (`token_budget`, `max_files`,
     `include_related`, `min_relevance`, `deduplicate = true`, `include_docs`, `scope`).
  3. Use `project_hash` + `query_text` + `SearchMode` as the cache key, and cache the
    unfiltered pre-assembly hit pool so `scope`, `include_related`, `min_relevance`,
    `include_docs`, `max_files`, and `token_budget` can all be applied from the request
    after the cache hit.
  4. Unless `bypass_cache`: check `QueryCache` with the chosen key;
    on hit, set `cache_hit = true` and skip search.
  5. On miss: `search_hybrid` over a shared candidate pool (unfiltered), then apply
    request filters (`scope`/`include_docs`/`min_relevance`/`max_files`) and
    `ContextAssembler::assemble`.
    - If caching the pre-assembly pool, store only raw hits (not already filtered by scope/min/max).
  6. Map `ContextPackage` → the §9 JSON envelope:
     - `context[]`: `ContextChunk` → `{ file (rel_path), lines ("start-end"), symbol,
       type, language, relevance (relevance_score), source (search|related|dependency),
       reason, content }`.
     - `metadata`: `files_included`, `total_tokens`, `budget_used_pct`, `chunks_returned`,
       `chunks_deduplicated`, `search_time_ms`, `cache_hit`, `index_status` (5.8),
       `index_coverage_pct`, `result_confidence`, `budget_gap_reason`,
       `missing_context_warnings`, `suggested_action`, `clusters`.

## Approach

- Keep the handler thin (the Phase 4 contract): parse → cache check → search → assemble →
  shape JSON. The QUALITY lives in 5.5; this task just wires it and shapes the wire envelope.
- Reuse the existing `context_result_to_json` mapping as the basis for the `context[]`
  items, but drive `source`/`reason`/`relevance` from the real `ContextChunk` instead of
  the hardcoded `"search"` / `"Primary Phase 4 search result"` placeholders.
- Map the PRD enums to the §9 wire strings: `Confidence` → `"high"|"medium"|"low"`,
  `GapReason` → `"no_more_relevant"|"index_incomplete"|"threshold_filtered"` (matches the
  §9 example `"budget_gap_reason": "no_more_relevant"`), `clusters` → `[{ path, chunk_count,
  avg_relevance }]` (note the wire key is `path`, the struct field is `path_prefix`).
- **Remove the Phase 4 "deferred to Phase 5" warnings** in `context_results_to_json`
  (dedup/expansion/budget/cache are now real). `missing_context_warnings` should now carry
  only genuine gaps surfaced by the assembler (e.g. partial index, expansion caps hit).
- Honor `bypass_cache` by skipping the `QueryCache` lookup AND still populating the cache
  on the fresh result (so the next non-bypass query benefits).
- Preserve the existing error-as-JSON / never-panic / never-stdout contract: bad/missing
  args → `{ "status": "error", "error": message }`; log via `tracing`, never write stdout.
- Do NOT change the MCP tool schema or argument names — Phase 4 clients must keep working.

## Acceptance criteria

- [x] `get_context_for_prompt` returns the §9 `{ context, metadata }` envelope backed by
      the real `ContextPackage` — `chunks_deduplicated`, `total_tokens`, and
      `budget_used_pct` reflect actual dedup + two-pass budget allocation (not stubs).
- [x] `include_related = true` produces chunks with `source: "related"` in the output
      (real expansion, not the Phase 4 "accepted but deferred" warning).
- [x] `token_budget` is honored: a small budget yields fewer/truncated chunks and
      `budget_used_pct` within ±5% of target; the Phase 4 "does not trim to fit" warning
      is gone.
- [x] `bypass_cache = false` returns `cache_hit: true` on a repeated identical query;
      `bypass_cache = true` forces a fresh search (`cache_hit: false`) and refreshes the cache.
- [x] `result_confidence`, `budget_gap_reason`, `clusters`, `index_status`,
      `missing_context_warnings`, and `suggested_action` are emitted from the real
      `ContextPackage` and use the §9 wire strings.
- [x] The MCP tool schema/argument names are unchanged (no Phase 4 regression).
- [x] Bad/missing args → JSON error object; the handler never panics nor writes to stdout.
- [x] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test mcp::handlers::tests::get_context_uses_real_assembler
cargo test mcp::handlers::tests::get_context_include_related_emits_related_source
cargo test mcp::handlers::tests::get_context_honors_token_budget
cargo test mcp::handlers::tests::get_context_cache_hit_and_bypass
cargo test mcp::handlers::tests::get_context_emits_confidence_and_clusters
cargo test mcp::handlers::tests::get_context_rejects_bad_args_as_json_error
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- This task ends the Phase 4 naïveté: the §9 envelope shape was already stable at 4.8, so
  this is a body swap. Keep the wire fields byte-identical where possible to avoid
  breaking existing handler tests; only the *values* (and the dropped warnings) change.
- Use the fake-embedder test seam (as Phase 3/4 handler tests do) so handler tests run
  without a downloaded model.
- `index_status` comes from the IndexStatusTracker (5.8); during this task it may still be
  derived from `IndexHealthCache` as in Phase 4 — the full `partial`/`full` wiring is the
  5.8 + 5.13 integration concern. Accept whatever 5.5 emits for `IndexIncomplete`.
- Stage-5 integration (5.13) verifies the full end-to-end behavior (confidence-signaled,
  clustered, deduplicated) against the PRD §5.3 schema; this task only needs the handler
  to delegate correctly.
