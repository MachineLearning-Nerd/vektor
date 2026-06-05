# Task 5.2 — `Deduplicator::deduplicate` (overlap-aware chunk merge)

**Phase**: 5 — Context Assembly
**Task ID**: 5.2
**PRD reference**: Section 5.3 (Deduplicator)
**Roadmap stage**: Stage 2 / `v0.4.0`
**Effort estimate**: M
**Depends on**: 4.5
**Blocks**: 5.5

## Objective

Collapse overlapping chunks that point at the same region of a file (common when a
function was sub-chunked or a sliding window produced adjacent fragments) into a
single chunk, **without** merging two semantically distinct functions that merely
live next to each other. This is the v2.2 fix: merge only when overlap exceeds
**50% of the smaller chunk's line range**. Runs early in the assembly pipeline
(filter → **dedup** → expand → recency → feedback → budget; PRD §5.4) so downstream
budgeting counts each region once.

## Inputs (must exist before starting)

- The result list from task 4.5: `Vec<HybridResult>` (`src/search/hybrid.rs`), each
  carrying `rel_path: String`, `start_line: u64`, `end_line: u64`,
  `relevance_score: f32`, and `content: String`. **Content is already in memory** —
  dedup must NOT re-read files from disk (PRD §5.3 step 4).
- The Phase 5 in-memory `ContextChunk` (PRD §5.3) is the eventual carrier, but it
  does not exist yet. Spec the deduplicator to operate over the existing 4.5
  `HybridResult` (or a thin chunk struct equivalent), so 5.5 can call it on the
  search output before converting to `ContextPackage`. Keep the type parameter
  honest with what 4.5 actually produces — do not invent fields.

## Outputs (must exist after completion)

- A `Deduplicator` type (suggested module `src/context/dedup.rs`) exposing:
  - `Deduplicator::deduplicate(chunks: Vec<HybridResult>) -> Vec<HybridResult>`
    (or the chunk type 5.5 settles on). Returns the merged, still-ranked-by-relevance
    set; order of the final returned list is the assembler's concern, but dedup must
    not silently reorder by line as its observable output — keep the relevance order
    or document that 5.5 re-sorts.
- Internal helpers (private):
  - sort key `(rel_path, start_line)` for the co-location scan
  - `overlap_lines(a, b) -> u64` and `min_span(a, b) -> u64` for the ratio test
  - a merge that **keeps the higher-scoring chunk** and **extends the line range** to
    cover both (`start = min(starts)`, `end = max(ends)`), preserving the kept
    chunk's `content`/`symbol`/`score`.

## Approach

Per PRD §5.3 (Deduplicator), in order:

1. **Sort** chunks by `rel_path`, then `start_line`. Only chunks with the same
   `rel_path` are merge candidates — different files never merge.
2. **Overlap test** between two co-located chunks: compute overlapping lines
   `max(0, min(a.end, b.end) - max(a.start, b.start) + 1)` and the smaller chunk's
   span `min(a.end-a.start+1, b.end-b.start+1)`. Merge **only if**
   `overlap > 0.50 * smaller_span`. The 50%-of-the-*smaller*-chunk rule is the v2.2
   fix that stops two distinct co-located functions from being merged just because
   one barely touches the other.
3. **Merge:** keep the chunk with the higher `relevance_score`; extend its line range
   to `[min(start), max(end)]`. The kept chunk's `content` is retained as-is (PRD does
   not ask to splice the two bodies — the surviving higher-scoring chunk's content
   stands, with the widened line range recording the covered span). Document this
   content-handling choice in a code comment.
4. **No disk reads** — content comes from the in-memory results only.
- Implement as a single pass over the sorted list, comparing each chunk against the
  current merge accumulator for the same file (interval-coalesce style), so it is
  `O(n log n)` from the sort. Reset the accumulator on file boundary.
- Edge cases: identical chunks (same id / same range) collapse to one; fully-nested
  chunk (one inside another) → overlap = smaller span = 100% > 50% → merge; adjacent
  but non-overlapping chunks (`a.end + 1 == b.start`) → overlap 0 → keep both.

## Acceptance criteria

- [x] Two chunks in the same file with **>50% overlap of the smaller** are merged into
      one whose range covers both and whose surviving content/score is the
      higher-scoring chunk's (the Phase 5 exit-criterion: 80%-overlap sliding window →
      merged chunks).
- [x] Two co-located chunks with **≤50% overlap of the smaller** are NOT merged (v2.2
      fix — semantically distinct neighbors survive separately).
- [x] Chunks in **different files** are never merged even if line ranges coincide.
- [x] A fully-nested chunk is merged into its container (overlap = 100% of smaller).
- [x] Adjacent, non-overlapping chunks (`end+1 == start`) are both kept.
- [x] No file is re-read from disk during dedup (content sourced from in-memory
      results only — assert via a fixture whose paths do not exist on disk).
- [x] Empty input → empty output, no panic.
- [x] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test context::dedup::tests::merges_high_overlap_chunks
cargo test context::dedup::tests::keeps_distinct_low_overlap_neighbors
cargo test context::dedup::tests::never_merges_across_files
cargo test context::dedup::tests::nested_chunk_is_absorbed
cargo test context::dedup::tests::adjacent_non_overlapping_kept
cargo test context::dedup::tests::no_disk_read_uses_in_memory_content
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- **Why "smaller chunk" and not "either chunk":** anchoring the 50% ratio to the
  smaller span is the precise v2.2 wording. A large container that overlaps a small
  fragment by 50% of the *small* fragment merges; the same absolute overlap measured
  against the *large* chunk would not trip 50% and would wrongly keep duplicates.
  Implement the ratio against the smaller span exactly.
- **Line indexing:** `HybridResult` uses inclusive `start_line`/`end_line` (u64).
  Use inclusive arithmetic (`end - start + 1` for span) consistently; off-by-one here
  changes the overlap ratio and the exit-criterion test outcome.
- **Type churn:** if 5.5 introduces `ContextChunk` before this lands, make
  `deduplicate` generic over a small trait (`rel_path`, `start_line`, `end_line`,
  `relevance_score`) or operate on `ContextChunk` directly — but only after that type
  exists. Until then, target `HybridResult` so the task is implementable standalone.
- **Score ties:** when two merge candidates have equal `relevance_score`, keep the one
  with the wider span (more context) or the earlier `start_line` — pick one and
  document it so the test is deterministic.
