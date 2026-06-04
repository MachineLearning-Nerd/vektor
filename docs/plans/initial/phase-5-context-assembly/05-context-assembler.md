# Task 5.5 — `ContextAssembler::assemble` orchestrator

**Phase**: 5 — Context Assembly
**Task ID**: 5.5
**PRD reference**: Section 5.3 (ContextAssembler — `AssemblyConfig` / `ContextPackage` / `ContextChunk` / `Confidence` / `GapReason` / `ResultCluster` structs), Section 5.4 (Token Budget Allocation Strategy), Section 12 Function CA.x
**Roadmap stage**: Stage 2 / v0.4.0
**Effort estimate**: L
**Depends on**: 5.1, 5.2, 5.3, 5.4, 5.9
**Blocks**: 5.6

## Objective

Build the orchestrator that turns raw `search_hybrid` output into a token-budgeted,
deduplicated, relationship-aware `ContextPackage`. This is the function `handle_get_context_for_prompt`
(5.6) calls. It composes the Phase 5 primitives — `TokenCounter` (5.1), `Deduplicator`
(5.2), `RelatedExpander` (5.3), `QueryCache` (5.4) — and the `RecencyTracker` (5.9), in
the exact pipeline order the PRD §5.4 lays out, and emits the full PRD §5.3 package
including confidence signaling, budget-gap reason, and directory clusters.

## Inputs (must exist before starting)

- `search_hybrid` results as `Vec<HybridResult>` (4.5): each carries `chunk_id`,
  `rel_path`, `start_line`/`end_line`, `symbol_name`, `symbol_type`, `language`,
  `content`, and a higher-is-better `relevance_score`.
- `TokenCounter::estimate` (5.1) — language-specific bytes/N heuristic plus the
  `tiktoken-rs` two-pass precise verification.
- `Deduplicator::deduplicate` (5.2) — sort by path+line, merge only if >50% overlap.
- `RelatedExpander::expand` (5.3) — chunk-level expansion with 0.6x/0.4x tiered
  scoring, expansion caps (max 5 files, 3 chunks/file), and hub-file skip (>20 imports).
- `RecencyTracker::score` (5.9) — mtime-based multiplier (1.1x @24h / 1.03x @7d / 1.0x
  older) with the >0.3 min-score gate.
- The `AssemblyConfig` / `ContextPackage` / `ContextChunk` / `Confidence` / `GapReason`
  / `ResultCluster` struct shapes from PRD §5.3 (use `relevance_score` / `rel_path` /
  `lines: (usize, usize)` internally — the §9 wire renaming happens in 5.6).

## Outputs (must exist after completion)

- `ContextAssembler::assemble(results: Vec<HybridResult>, config: &AssemblyConfig) -> Result<ContextPackage>`
  (recency + expansion need the store/recency tracker, so pass them in via the
  assembler struct or as additional params — keep the signature internal-crate).
- The PRD §5.3 structs realized in `src/context/` (e.g. `mod.rs` + `budget.rs`):
  - `AssemblyConfig { token_budget, max_files, include_related, min_relevance, deduplicate, include_docs, scope }`.
  - `ContextPackage { chunks, files_included, total_tokens, budget_used_pct,
    missing_context_warnings, search_metadata, result_confidence, budget_gap_reason,
    suggested_action, clusters }`.
  - `ContextChunk { content, rel_path, lines, symbol, relevance_score, source, reason }`
    with `ChunkSource = Search | Related | Dependency`.
  - `enum Confidence { High, Medium, Low }`, `enum GapReason { NoMoreRelevant,
    IndexIncomplete, ThresholdFiltered }`, `struct ResultCluster { path_prefix,
    chunk_count, avg_relevance }`.

## Approach

Implement the PRD §5.4 pipeline in order (search itself happens in the caller; the
assembler receives the result pool):

1. **Filter (min_relevance):** drop search-origin chunks below `config.min_relevance`.
   Track whether any chunks were filtered (feeds `ThresholdFiltered`).
2. **Deduplicate (5.2):** when `config.deduplicate`, merge overlapping chunks (>50%
   overlap of the smaller range); keep the higher-scoring chunk, extend the line range.
   Record the dedup count for metadata.
3. **Expand related (5.3):** when `config.include_related`, call `RelatedExpander::expand`
   — chunk-level, tiered 0.6x/0.4x, caps + hub-skip enforced inside 5.3. Expanded
   chunks are tagged `source = Related` and are **exempt from min_relevance** (PRD §5.3 v2.2).
4. **Apply recency (5.9):** `final_score = relevance_score * recency_weight`, with the
   >0.3 min-score gate so a recently-edited but irrelevant file does not pollute top-5.
   (Feedback multiplier from PRD §5.4 step 6 is a later task — note it, do not implement.)
5. **Greedy token-budget allocation with two-pass verify (5.1, PRD §5.4 + §5.3 v2.3):**
   sort by `final_score` desc; greedily include chunks (fast heuristic estimate) until
   ~90% of `token_budget`; then run the precise `tiktoken-rs` count over the assembled
   package — if under, add more chunks; if over, truncate the last chunk to fit (break
   if even a truncated chunk won't fit). Enforce `max_files` distinct files.
6. **Assemble the `ContextPackage`:** compute `files_included`, `total_tokens` (precise
   count), `budget_used_pct`, and the signaling fields below.

Signaling/derived fields:
- `result_confidence` (PRD §5.3 heuristic): `High` = top score >0.8 AND ≥3 results above
  `min_relevance`; `Medium` = top score 0.5–0.8; `Low` = top score <0.5 OR <2 above
  threshold.
- `budget_gap_reason` (only when budget wasn't fully used): `NoMoreRelevant` (remaining
  chunks below `min_relevance`), `IndexIncomplete` (index status not `full` — passed in
  from the caller), `ThresholdFiltered` (chunks existed but were filtered out in step 1).
- `suggested_action`: agent guidance string, e.g. `Low` confidence → "try a broader query".
- `clusters`: group included chunks by directory `path_prefix`; emit `path_prefix`,
  `chunk_count`, `avg_relevance`. Populate when results span >2 distinct prefixes
  (matches §9's "appears when results span >2 distinct code areas").
- `missing_context_warnings`: surface honest gaps (e.g. partial index, dedup/expansion
  applied) for the agent.

Keep the assembler pure-ish: content is carried from search results already in memory,
NOT re-read from disk (PRD §5.3 Deduplicator point 4). Greedy alloc must stay <5ms.

## Acceptance criteria

- [ ] `assemble` runs the pipeline in PRD §5.4 order: filter → dedup → expand → recency
      → greedy budget alloc with two-pass `tiktoken-rs` verify.
- [ ] Token budget: a `token_budget=8000` request produces a package within ±5% of the
      target (two-pass verification working; the phase exit criterion).
- [ ] Dedup: a sliding-window fixture with ~80% overlap yields merged chunks; co-located
      chunks with <50% overlap are NOT merged.
- [ ] Related expansion: when `include_related`, expanded chunks appear tagged
      `source = Related`, are exempt from the `min_relevance` floor, and respect the
      5.3 caps (≤5 files, ≤3 chunks/file) and hub-file skip.
- [ ] Recency: a recently-edited file outranks an older file with the same RRF score,
      but a recently-edited *irrelevant* file stays out of top-5 (min-score gate >0.3).
- [ ] `result_confidence` follows the PRD §5.3 heuristic (High/Medium/Low) on known inputs.
- [ ] `budget_gap_reason` is `None` when budget is fully used; otherwise the correct
      `NoMoreRelevant` / `IndexIncomplete` / `ThresholdFiltered` variant.
- [ ] `clusters` groups chunks by directory prefix with `chunk_count` + `avg_relevance`.
- [ ] `ContextPackage` matches the PRD §5.3 struct (all fields populated, no stubs).
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test context::tests::assemble_pipeline_order
cargo test context::tests::token_budget_within_five_percent
cargo test context::tests::dedup_merges_high_overlap_only
cargo test context::tests::related_expansion_exempt_from_min_relevance
cargo test context::tests::recency_boost_respects_min_score_gate
cargo test context::tests::confidence_heuristic_high_medium_low
cargo test context::tests::budget_gap_reason_variants
cargo test context::tests::clusters_group_by_directory
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- **Confidence calibration is heuristic-only at v0.4** (PRD §5.3 + phase README): the
  High/Medium/Low rule here is the shipped behavior. Data-driven calibration against a
  labeled corpus is task C2 (Stage 5, post-v1.0) — do not attempt it here.
- **Feedback multiplier (PRD §5.4 step 6) is deferred** — the `FeedbackStore` is a later
  task. Leave a clean seam (`final_score *= feedback_multiplier` defaulting to 1.0) but
  do not wire SQLite here.
- **Two-pass verify adds <1ms** (PRD §5.3 v2.3); keep the fast heuristic for the 90% fill
  pass and only run the precise count once on the assembled set, not per-chunk.
- `index_status` is owned by the IndexStatusTracker (5.8) and the caller (5.6); the
  assembler only consumes it to decide `IndexIncomplete`. Accept it as an input, do not
  read it from disk inside `assemble`.
- Reuse the existing `HybridResult` → chunk shaping already prototyped in
  `src/mcp/handlers.rs` (`context_result_to_json`) so 5.6's wire mapping stays trivial.
