# Task 5.9 — `RecencyTracker::score(base_score, mtime)` — recency-weighted ranking

**Phase**: 5 — Context Assembly
**Task ID**: 5.9
**PRD reference**: Section 5.5 (Recency-Weighted Ranking)
**Roadmap stage**: Stage 2 / v0.4.0
**Effort estimate**: S
**Depends on**: 4.5
**Blocks**: 5.5

## Objective

Nudge recently-edited files up the ranking — a tiebreaker for active-development
relevance, not a ranking override. PRD §5.5 (v2.2 fix) reduced the boost from the
original 1.3× to a gentle 1.1×/1.03× and added a min-score gate so a recently
saved but irrelevant file cannot be boosted into the top results.

## Inputs (must exist before starting)

- A per-chunk `last_modified` Unix-epoch timestamp. This already exists on
  `VectorStore::SearchResult.last_modified` (`i64`, seconds) and on `HybridResult`'s
  upstream rows — so the mtime is in hand at ranking time; no extra disk stat.
- The base relevance score to weight — the RRF / hybrid score from 4.5
  (`HybridResult.relevance_score`), higher-is-better.
- A "now" reference (Unix epoch seconds) — injectable so the boost is testable
  without depending on the wall clock.

## Outputs (must exist after completion)

- `RecencyTracker::score(&self, base_score: f32, mtime: i64) -> f32` returning the
  recency-weighted score. The multiplier tiers (v2.2):
  - Modified **< 24h** ago → **1.1×**
  - Modified **< 7d** ago → **1.03×**
  - Older (or future/unknown mtime) → **1.0×**
- **Min-score gate (v2.2):** apply the multiplier ONLY when `base_score > 0.3`;
  otherwise return `base_score` unchanged. Formula (PRD §5.5):
  `final = base_score * (if base_score > 0.3 { multiplier } else { 1.0 })`.
- A constructor that takes (or defaults to) the current time, so 5.5 can hold one
  `RecencyTracker` and apply it across a result set with a single consistent `now`.

## Approach

- Compute `age = now - mtime` (seconds). Map to a multiplier:
  `age < 86_400` → 1.1; `age < 604_800` → 1.03; else 1.0. Treat a negative age
  (mtime in the future, e.g. clock skew) as "older" → 1.0, never a boost.
- Gate on `base_score > 0.3` before multiplying; at or below the gate, pass the
  base score through untouched.
- Pure function over `(base_score, mtime, now)` — no I/O. 5.5 calls it inside the
  scoring loop (PRD §5.4 step 5: `final_score = rrf_score * recency_weight`),
  before feedback adjustment and budget allocation.

## Acceptance criteria

- [ ] A file modified < 24h ago with `base_score = 0.8` scores `0.8 * 1.1`.
- [ ] A file modified between 24h and 7d ago gets the 1.03× tier.
- [ ] A file older than 7d (and a future-dated mtime) gets 1.0× (unchanged).
- [ ] Min-score gate: `base_score = 0.2` with a < 24h mtime returns `0.2`
      unchanged — a recently-edited irrelevant file is NOT boosted (phase exit
      criterion: it must not pollute top-5).
- [ ] Boundary at `base_score == 0.3` is handled per the `> 0.3` rule (0.3 itself
      is not boosted) and is covered by a test.
- [ ] `score` is pure and deterministic for a fixed `now` (injected clock).
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test context::recency::tests::boost_within_24h
cargo test context::recency::tests::boost_within_7d
cargo test context::recency::tests::older_and_future_unchanged
cargo test context::recency::tests::min_score_gate_blocks_irrelevant
cargo test context::recency::tests::gate_boundary_at_0_3
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- **mtime source**: PRD §5.5 says when Phase 2 git-blame data is available, use
  the last commit timestamp instead of file mtime for more accurate per-chunk
  recency. For v0.4.0 use the `last_modified` already stored per chunk in LanceDB;
  the git-blame upgrade is a later, signature-compatible swap.
- Keep the tier thresholds as named constants (`SECS_24H`, `SECS_7D`) so the
  values stay auditable against the PRD and are easy to tune.
- The 0.3 gate must use the SAME score orientation as 4.5's `relevance_score`
  (higher-is-better). Do not feed raw L2 distance into this function.
- No NEW Cargo.toml deps — pure arithmetic over stdlib types.
