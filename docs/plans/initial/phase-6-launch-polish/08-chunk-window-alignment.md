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

- [ ] Chunk token-length distribution per model is measured and documented.
- [ ] A decision between (a)/(b)/(c) is recorded with the retrieval-benchmark evidence that justified it.
- [ ] Implementation matches the recorded decision; if (a) or (c), sub-chunking preserves the v2.3 header rule.
- [ ] Retrieval benchmark (Precision@5 / Recall@5 / MRR on the 6.3 fixture) does not regress; deltas are posted in `BENCHMARKS.md`.
- [ ] `LITE=1 scripts/scale-test.sh` passes with `failed: 0` and the re-embed migration cost is stated.

## Verification

```bash
cargo test --workspace
cargo bench --bench retrieval_quality
./scripts/ci.sh
LITE=1 ./scripts/scale-test.sh
```
