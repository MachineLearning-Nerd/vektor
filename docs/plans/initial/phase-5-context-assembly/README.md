# Phase 5 — Context Assembly (`v0.4.0`)

> **Goal**: The differentiator. `get_context_for_prompt` returns token-budgeted, deduplicated, relationship-aware context packages. Shallow indexing makes the first response available in <5s on a new project. Recency weighting and adaptive hybrid weights produce quality results that beat raw search.

**Roadmap mapping**: Stage 2 / `v0.4.0`
**PRD mapping**: Section 5 (Context Assembly Layer) in full, Section 12 Functions CA.1–CA.12
**Effort estimate**: 3–4 weeks of focused part-time work
**Status**: ⬜ Implementation not started — **per-task files now written** (`01`–`09`, `12`, `13`; tasks 5.10/5.11 are folded into 4.4 and have no file). Implementation starts from the linked task files.

---

## Task list

| ID | Task | Effort | Depends on | Status |
|---|---|---|---|---|
| 5.1 | `TokenCounter::estimate` — language-specific bytes/N ratios + `tiktoken-rs` two-pass verification | S | 4.5 | ⬜ |
| 5.2 | `Deduplicator::deduplicate` — sort by path+line, merge only if >50% overlap (PRD §5.3 v2.2 fix) | M | 4.5 | ⬜ |
| 5.3 | `RelatedExpander::expand` — chunk-level expansion, tiered scoring (0.6x/0.4x), hub-file skip, expansion caps | L | 4.5 | ⬜ |
| 5.4 | `QueryCache` — LRU + file-level invalidation (per PRD §5.3 v2.2 fix), 60s TTL | M | 4.5 | ⬜ |
| 5.5 | `ContextAssembler::assemble` — orchestrate filter → dedup → expand → recency → feedback → budget greedy alloc + two-pass tiktoken verify | L | 5.1, 5.2, 5.3, 5.4 | ⬜ |
| 5.6 | `handle_get_context_for_prompt` — replace Phase 4's naive handler with the full assembly pipeline | M | 4.8, 5.5 | ⬜ |
| 5.7 | `ShallowIndexer::build` — file-walk + Tantivy index from path + first 50 lines + last 20 lines + regex-extracted decls (PRD §4.4) | M | 2.6, 4.1 | ⬜ |
| 5.8 | `IndexStatusTracker` — project-scoped `IndexPhase` shared state; handlers consult before search | S | 5.7 | ⬜ |
| 5.9 | `RecencyTracker::score` — mtime-based, 1.1x at 24h / 1.03x at 7d / 1.0x older, min-score gate >0.3 (PRD §5.5 v2.2 fix) | S | 4.5 | ⬜ |
| 5.10 | (Folded into 4.4) — `SynonymExpander` static map | — | (done in 4.4) | ⬜ |
| 5.11 | (Folded into 4.4) — `AdaptiveWeights::compute` density-based selection | — | (done in 4.4) | ⬜ |
| 5.12 | `WarmUp::run` — embed dummy strings at batch_size=1 AND batch_size=32 at server startup (PRD §6.1 v2.2 fix) | S | 3.2 | ⬜ |
| 5.13 | Stage-5 integration: end-to-end `get_context_for_prompt` returns confidence-signaled, clustered, deduplicated results matching PRD §5.3 schema | M | 5.6, 5.8, 5.12 | ⬜ |

---

## Phase exit criteria

All must be true before moving to Phase 6:

- [ ] All 13 tasks above marked ✅ Done (5.10/5.11 are folded into 4.4 — they're listed for cross-reference only)
- [ ] `get_context_for_prompt` returns the full `ContextPackage` JSON per PRD §5.3 with: `chunks`, `files_included`, `total_tokens`, `budget_used_pct`, `missing_context_warnings`, `result_confidence`, `budget_gap_reason`, `clusters`
- [ ] Token budget: requesting `token_budget=8000` produces a response within ±5% of the target (two-pass verification working)
- [ ] Dedup: a sliding-window fixture with 80% overlap produces merged chunks
- [ ] Related expansion: a query that hits `src/auth/jwt.py` includes chunks from `tests/test_auth.py` if such a test file exists
- [ ] Recency: a recently-edited file ranks higher than an older file with the same RRF score; but a recently-edited *irrelevant* file does NOT pollute top-5 (min-score gate)
- [ ] Shallow indexing: a fresh `vektor index` returns BM25 results within 5s, before deep indexing completes
- [ ] IndexStatusTracker: while shallow indexing runs, `get_context_for_prompt` returns `index_status: "partial"` with results; after deep completes, returns `"full"`
- [ ] Warm-up: first query after `vektor serve` start completes in <150ms (not the 3-5s cold start)
- [ ] CI green; no regression on Phase 4 search latency

---

## Notes

- **This phase is THE differentiator.** Per the PRD, "Augment Code's 30-80% agent performance improvement comes from this layer, not from better embeddings." Get this right.
- **Two-pass token budget**: greedy fill to 90% with the fast heuristic, then verify with `tiktoken-rs` and truncate if over (PRD §5.3 v2.3 fix). Adds <1ms; saves on budget overflow.
- **Chunk-level expansion** (not file-level — PRD §5.3 v2.3 fix): when expanding to a related file, run a quick vector similarity check on that file's chunks and include only those scoring >0.3. Prevents a 500-line middleware file from consuming the token budget when only 20 lines are relevant.
- **Hub-file detection**: skip files with >20 inbound or outbound imports during expansion. These are usually `index.ts` / `mod.rs` re-export barrels — meaningless context.
- **Confidence calibration is Phase 5 work (heuristic) + Stage 5 polish (data-driven)**: at v0.4 we use the heuristic from PRD §5.3 (`top score >0.8 + ≥3 above threshold = High`). Real calibration against the labeled corpus is C2 (Stage 5 of the roadmap, post-v1.0).
- **Watch the test surface**: this phase has the most behavior to verify. Plan ≥1 integration test per task (13+ tests total). Tests are the only way to know dedup, expansion, recency, and budget all compose correctly.

---

## When this phase completes

1. Mark all tasks ✅
2. (No tag yet — release happens after Phase 6)
3. Update Current State tables
4. Expand `phase-6-launch-polish/` from task list to per-task files
