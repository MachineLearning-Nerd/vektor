# Task 5.13 — Stage-5 integration: end-to-end `get_context_for_prompt`

**Phase**: 5 — Context Assembly
**Task ID**: 5.13
**PRD reference**: Section 5.3 (`ContextPackage` shape) + the phase-5 README exit criteria
**Roadmap stage**: Stage 2 / v0.4.0
**Effort estimate**: M
**Depends on**: 5.6, 5.8, 5.12
**Blocks**: 6.*

## Objective

Prove the Phase 5 differentiator works end to end and close the phase. This task
adds the integration tests that drive `get_context_for_prompt` through the full
assembly pipeline (5.6) over a real fixture project, and asserts each phase-5
exit criterion: the response is a confidence-signaled, clustered, deduplicated
`ContextPackage` per PRD §5.3; the token budget lands within ±5% of target via
two-pass verification; dedup, related expansion, and recency all compose
correctly; the two-tier index surfaces BM25 results in <5s then flips
`index_status` partial→full; and the warmed first query completes in <150ms —
with no regression on Phase 4 search latency.

## Inputs (must exist before starting)

- `handle_get_context_for_prompt` rebuilt on the full assembly pipeline (5.6 →
  `ContextAssembler::assemble`, 5.5).
- `IndexStatusTracker` (5.8) shared across handlers.
- `WarmUp::run` (5.12) wired into `vektor serve` startup.
- The component tasks the pipeline composes: `TokenCounter` (5.1),
  `Deduplicator` (5.2), `RelatedExpander` (5.3), `QueryCache` (5.4),
  `RecencyTracker` (5.9), and `ShallowIndexer` (5.7).
- The PRD §5.3 `ContextPackage` shape: `chunks`, `files_included`,
  `total_tokens`, `budget_used_pct`, `missing_context_warnings`,
  `search_metadata`, `result_confidence`, `budget_gap_reason`,
  `suggested_action`, `clusters`.

## Outputs (must exist after completion)

- An end-to-end integration test (likely `tests/stage5_context.rs` or
  `tests/context_assembly.rs`) that builds a fixture project, indexes it
  (shallow then deep), and calls `get_context_for_prompt`, asserting the full
  `ContextPackage` JSON per PRD §5.3 — every field present and well-formed:
  `chunks`, `files_included`, `total_tokens`, `budget_used_pct`,
  `missing_context_warnings`, `result_confidence`, `budget_gap_reason`,
  `clusters`.
- Test coverage for each exit criterion (see Acceptance criteria), using a
  deterministic fake embedder where a real model would make the test
  model-gated, plus `#[ignore]`d model-backed variants for the latency
  assertions (mirroring the Phase 3/4 convention).
- A fixture set: an 80%-overlap sliding-window pair (dedup), a result file plus
  its matching test file (expansion), a recently-`mtime`d relevant file and a
  recently-`mtime`d irrelevant file (recency + min-score gate).
- The phase-5 completion checklist actioned: mark all tasks ✅, update Current
  State tables, expand `phase-6-launch-polish/` from task list to per-task files
  (per the README "When this phase completes" steps; no release tag yet).

## Approach

- Drive the assembly pipeline at the handler boundary (the same JSON contract
  agents call), not the internal structs, so the test proves the real wire shape.
- Budget: request `token_budget=8000` and assert `total_tokens` lands within ±5%
  of target — exercises the 5.1 two-pass `tiktoken-rs` verification.
- Dedup: feed an 80%-overlap sliding-window fixture; assert the overlapping pair
  is merged into one chunk (5.2 >50%-overlap rule).
- Expansion: a query that hits a source file (PRD example `src/auth/jwt.py`)
  must pull in chunks from a matching test file (`tests/test_auth.py`) when one
  exists (5.3 chunk-level expansion).
- Recency: a recently-edited *relevant* file ranks above an older file with the
  same RRF score (5.9 1.1x@24h boost); a recently-edited *irrelevant* file must
  NOT enter top-5 (the >0.3 min-score gate blocks it).
- Two-tier: a fresh `vektor index` returns BM25 results in <5s (shallow, 5.7),
  and `index_status` reads `"partial"` during shallow then flips to `"full"`
  after deep (5.8). Use a generous time bound for the <5s assertion.
- Warm query: after warm-up (5.12), the first query completes in <150ms — assert
  in a model-backed `#[ignore]`d test (the warm path needs the real session).
- Latency guard: assert no regression versus the Phase 4 search-latency
  baseline; CI must stay green.

## Acceptance criteria

- [ ] `get_context_for_prompt` returns the full PRD §5.3 `ContextPackage` JSON:
      `chunks`, `files_included`, `total_tokens`, `budget_used_pct`,
      `missing_context_warnings`, `result_confidence`, `budget_gap_reason`,
      `clusters` all present.
- [ ] `token_budget=8000` yields `total_tokens` within ±5% of target (two-pass).
- [ ] An 80%-overlap sliding-window fixture produces merged chunks.
- [ ] A query hitting a source file includes chunks from its matching test file
      when one exists.
- [ ] A recent relevant file outranks an older same-score file; a recent
      irrelevant file is kept out of top-5 by the min-score gate.
- [ ] Fresh `vektor index` returns BM25 results within 5s; `index_status` reads
      `"partial"` during shallow and `"full"` after deep.
- [ ] First query after warm-up completes in <150ms (model-backed `#[ignore]`d).
- [ ] CI green with no Phase 4 search-latency regression.
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test --test stage5_context          # end-to-end ContextPackage + exit-criteria assertions
cargo test context                         # assembly component tests still green
cargo test mcp::handlers                   # handler contract preserved
cargo test search                          # Phase 4 search latency baseline — no regression
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
# Model-gated (manual / #[ignore]d, after `vektor models download`):
#   warm-query <150ms; deep-index quality; index twice → 0 re-embeds.
```

## Notes / open questions

- **This is the phase-closing task.** Passing it triggers the README "When this
  phase completes" checklist: mark all tasks ✅, update Current State tables, and
  expand `phase-6-launch-polish/` to per-task files. No release tag yet — the
  release happens after Phase 6.
- Confidence calibration is the §5.3 *heuristic* at v0.4 (`top score >0.8 + ≥3
  above threshold = High`); data-driven calibration is Stage 5 (C2, post-v1.0).
  Assert the heuristic here, not calibrated accuracy.
- Latency assertions (<150ms warm query, <5s shallow) are environment-sensitive;
  follow the Phase 3/4 convention of `#[ignore]`d model-backed tests for the
  ones that need the real model, and generous bounds for CI-run timing tests.
- Plan ≥1 integration test per behavior (the README flags this phase as the
  heaviest test surface — 13+ tests total across the phase).
