# Task 6.8 — Chunk size vs embedder window alignment

## Objective

Decide and implement how chunk sizing interacts with the active embedder's maximum sequence length. Today's 200-line AST chunks tokenize far past bge-small's 512-token window, so after the truncation fix (`0d222b0`) the lite model embeds only the head (~first 40–60 lines) of any large function — the tail is invisible to semantic search. The PRD's chunk sizing (§v2.2, 200-line max) implicitly assumed Jina v2's 8192-token window.

## Scope

- Quantify the problem first: measure the token-length distribution of chunks on the tokio corpus per model tokenizer (what fraction truncates at 512? at 8192?).
- Evaluate the options, using the 6.3 retrieval benchmark as the quality judge:
  - (a) Per-model chunk token budget: sub-chunk when a chunk exceeds the active model's window, reusing the existing v2.3 header-preservation rule (parent signature prepended to each sub-chunk).
  - (b) Accept head-only embeddings for the lite model and document the trade-off (BM25 still covers the tail; hybrid search may mask the loss).
  - (c) Embed oversized chunks as multiple windows and mean-pool into one vector (no row-count change, extra embed cost).
- Record the decision PRD-style (amend §chunking or add a short ADR note in `docs/`) — this is a design decision, not just a patch.
- Account for the migration cost: any chunking change alters `content_hash` for affected chunks, invalidating the embedding-reuse cache and forcing re-embeds on next index. Call this out in release notes if it ships in a tagged version.
- Non-goal: changing the default model or the 200-line PRD cap for Jina; this task is about the lite/small-window path.

## Acceptance Criteria

- [x] Chunk token-length distribution per model is measured and documented (see Decision below; harness: `chunk_token_distribution_profile`, `#[ignore]`).
- [x] A decision between (a)/(b)/(c) is recorded with the evidence that justified it — (b), from the token-distribution measurement; the 6.3 fixture is BM25-only and could not have judged an embedding-side change.
- [x] Implementation matches the recorded decision: no chunking change; truncated rows made observable via `rows_at_cap` in the `embed_batch` telemetry span.
- [x] Retrieval benchmark cannot regress — no chunking or indexing behavior changed (noted here in lieu of a `BENCHMARKS.md` delta).
- [x] No re-embed migration cost: chunk boundaries and `content_hash` values are unchanged.

## Decision (closed 2026-07-12): (b) — keep 200-line chunks, accept head-only on small-window models

Measured on tokio (864 files) with the bge-small tokenizer, no truncation,
across `chunk_max_lines` candidates:

| max_lines | chunks | p50 | p90 | max | >512 tokens | tokens lost @512 |
|---|---|---|---|---|---|---|
| 200 (current) | 8,323 | 76 | 373 | 2,511 | 6.2% | 11.1% |
| 64 | 8,422 | 77 | 385 | 2,511 | 6.4% | 9.6% |
| 48 | 8,562 | 79 | 384 | 2,511 | 5.6% | 8.4% |
| 40 | 8,738 | 82 | 372 | 2,511 | 4.7% | 7.9% |
| 32 | 9,061 | 86 | 339 | 2,511 | 4.1% | 7.5% |

Why (b):

- **Option (a) — line-budget sub-chunking — is refuted by the data.** A 6x
  cut in line budget (200 → 32) reduces lost tokens only 11.1% → 7.5%
  while adding 9% more chunks and 8% more embed compute. The over-512
  tail is driven by token-DENSE lines (macro-heavy code, long literals,
  doc tables — note `max=2511` survives every line budget), which
  line-based splitting cannot address. Fixing it for real would need
  token-aware chunk boundaries, making chunk identity model-dependent —
  a content-hash / cross-model-compatibility cost far out of proportion
  to a 6% tail.
- **The default model is unaffected entirely**: the largest chunk is
  2,511 bge-tokens, far under Jina v2's 8,192 window. Head-only
  embedding is a lite-model-only trade-off.
- **The tail keeps keyword coverage**: Tantivy indexes full chunk content
  regardless of embedding truncation, so hybrid search still reaches
  truncated tails through the BM25 path.
- **Option (c) — window-pooling — deferred**, not rejected: it targets
  exactly the 6.2% tail with no row/hash changes (+~12% embed compute).
  Revisit if lite becomes the recommended first-run path at launch or if
  real-repo semantic-quality evals (needle queries) show tail misses.

Observability shipped instead of machinery: `embed_batch` logs
`rows_at_cap` (rows at the truncation bound) so the trade-off is visible
in any `-vv` index run rather than silent.

## Verification

```bash
cargo test --workspace
cargo bench --bench retrieval_quality
./scripts/ci.sh
LITE=1 ./scripts/scale-test.sh
```
