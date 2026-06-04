# Task 5.12 — `WarmUp::run` — eliminate ONNX cold start at `vektor serve`

**Phase**: 5 — Context Assembly
**Task ID**: 5.12
**PRD reference**: Section 6.1 (Backend Comparison + ONNX warm-up, v2.2 fix)
**Roadmap stage**: Stage 2 / v0.4.0
**Effort estimate**: S
**Depends on**: 3.2
**Blocks**: 5.13

## Objective

Pay the ONNX session's lazy-allocation cost at server startup instead of on the
first real query. PRD §6.1 records that local Jina v2 inference is ~200-400ms per
batch of 32, but the *first* forward pass after process start incurs an
additional 3-5s cold start (lazy tensor/EP allocation). `WarmUp::run(embedder)`
embeds dummy strings at `vektor serve` startup so the **first real
`get_context_for_prompt` / `search_code` query completes in <150ms** rather than
eating the 3-5s cold start. The v2.2 fix is to warm **both** batch code paths —
batch_size=1 and batch_size=32 — because each shape triggers its own lazy
allocation.

## Inputs (must exist before starting)

- The `Embedder` trait (3.2, `src/embedder/mod.rs`) with `embed` /
  `embed_documents` / `embed_query` and `dim()`.
- `OnnxEmbedder` (3.2, `src/embedder/onnx.rs`). Note: its *constructor* already
  performs an internal `warm_up` at batch 1 and 32 (`MAX_BATCH_SIZE = 32`) when
  the session is built. This task adds a backend-agnostic, server-startup warm-up
  over the `Embedder` trait object so the `vektor serve` path is explicit and
  testable, and so non-ONNX backends are handled uniformly (see Notes).
- The `vektor serve` bootstrap (1.6 / 4.7) where engine state is constructed.

## Outputs (must exist after completion)

- `WarmUp::run(embedder: &dyn Embedder) -> Result<()>` (or
  `async fn run(embedder: &Arc<dyn Embedder>)` matching the server's handle
  type) that:
  - embeds a 1-element dummy batch (warms the batch_size=1 path),
  - embeds a 32-element dummy batch (warms the batch_size=32 path),
  - uses the trait's document path so the real prefix is applied (no cold prefix
    path on first query),
  - discards the vectors (only the session priming matters),
  - logs warm-up completion + elapsed time via `tracing` to stderr (never
    stdout — stdio is the MCP transport).
- A call site in the `vektor serve` startup that runs `WarmUp::run` after the
  embedder is constructed and before the server starts accepting queries (eager
  warm-up pairs with eager engine state per the 4.7 lazy-vs-eager note).
- Warm-up failures are surfaced as a `Result` error (fail fast at startup), not
  silently swallowed.

## Approach

- Implement `WarmUp::run` against the `Embedder` trait so it is backend-agnostic
  and unit-testable with a fake embedder — do not reach into `OnnxEmbedder`
  internals.
- Build two dummy text batches: one of length 1, one of length 32 (the
  `MAX_BATCH_SIZE` the ONNX backend batches against). Short, fixed, ASCII dummy
  strings — content is irrelevant; the shapes are what prime the session.
- Call `embed_documents` (or `embed`) for each batch and drop the result.
- Wire the call into the `serve` startup sequence; on the lazy-engine path,
  warm-up runs when the engine is first constructed. Record the decision in 4.7.
- Time the warm-up with `std::time::Instant` and log at info level.

## Acceptance criteria

- [ ] `WarmUp::run` embeds a 1-element batch AND a 32-element batch (both code
      paths primed) using the `Embedder` trait.
- [ ] Warm-up uses the document embedding path so the prefix is applied (no
      first-query prefix cold path).
- [ ] Warm-up vectors are discarded; no state leaks into the index.
- [ ] A warm-up failure returns an `Err` and aborts startup (fail fast), and is
      logged via `tracing` to stderr — nothing on stdout.
- [ ] The `vektor serve` startup invokes `WarmUp::run` after the embedder is
      built and before serving queries (per the eager path in 4.7).
- [ ] A unit test drives `WarmUp::run` with a fake embedder and asserts both a
      length-1 and a length-32 batch reached the backend.
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test warmup                         # WarmUp::run drives batch 1 and 32 via a fake embedder
cargo test mcp::server                    # serve startup invokes warm-up (no stdout output)
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
# Model-gated (manual, after `vektor models download`): start `vektor serve`,
# issue the first search_code/get_context_for_prompt call, confirm <150ms (not 3-5s).
```

## Notes / open questions

- **Don't duplicate the in-constructor warm-up.** `OnnxEmbedder::new` already
  warms its own session at batch 1 and 32 (task 3.2). `WarmUp::run` is the
  *server-level, trait-level* hook: it makes the `serve` path explicit, covers
  any backend reached through `build_embedder`, and gives the integration test
  (5.13) a single place to assert the <150ms first-query property. For ONNX the
  trait-level warm-up is effectively a confirming second pass; for cloud backends
  it primes the HTTP client / connection.
- The <150ms first-query target is the warm result on the default local Jina v2
  backend; cloud backends have their own latency profile (PRD §6.1 table).
- Open question: on the lazy-open engine path (4.7), warm-up should run when the
  engine is first constructed, not at process start. Confirm the call site once
  4.7's lazy-vs-eager decision is finalized and note it here.
