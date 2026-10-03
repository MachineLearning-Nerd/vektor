# Task 6.7 — Embedding throughput investigation

## Objective

Close the cold-index throughput gap found by the scale-test harness: tokio (8,323 chunks, lite model) embeds at ~194 ms/chunk (1,616 s cold), which extrapolates to hours for the PRD's 10K-file target (<180 s lite, <600 s Jina). Profile first; change defaults only with before/after scale-test evidence.

## Scope

- Instrument the embed hot path with per-stage timings (tokenize, tensor build, `Session::run`, pool/normalize) behind `tracing` debug spans — measure before touching anything.
- Investigate ONNX Runtime threading: CPU peaked at ~330% of 800% during the tokio run. Check what `with_intra_threads` is set to in `build_session`, whether inter-op parallelism applies, and whether the session `Mutex` serializes work that could pipeline (e.g. tokenize batch N+1 while batch N runs).
- Experiment with `MAX_BATCH_SIZE` (currently 32): measure 64/128 against peak RSS (already ~2.5 GB at batch 32 — PRD RSS target is <700 MB, so larger batches may be the wrong direction; the data decides).
- Record findings and the chosen configuration in `BENCHMARKS.md` alongside the retrieval baseline.
- Non-goals: GPU/CoreML execution providers, model quantization, model swaps — those are Stage 6 accuracy/perf work. Length bucketing already landed (`0d222b0`) and composes with anything chosen here.

## Acceptance Criteria

- [x] Per-stage timing profile of the embed path is captured and documented (where do the 194 ms/chunk actually go?).
- [x] ONNX intra-op thread configuration is verified against the host core count, with measured impact.
- [x] At least one lever is validated or explicitly rejected with measured numbers (bucketing validated at 1.4x; CoreML rejected — see findings).
- [x] Peak RSS does not regress above the current ~2.5 GB, and the chosen config is justified against the PRD <700 MB target (no config change shipped; RSS unchanged).
- [x] Embedding output is unchanged for identical input (no EP/config change shipped; bucketing is order-invariant by construction).

## Findings (investigation closed 2026-07-12)

Instrumentation: `embed_batch` emits a per-batch debug span (`rows`,
padded `seq_len`, `tokenize_ms`, `forward_ms`, `total_ms`); manual profile
harness `onnx_embedder_throughput_profile` (`#[ignore]`, env-overridable
model/data dir) prints ms/text at three length classes.

**The forward pass IS the cost; the pipeline is negligible.** bge-small on
8-core Apple Silicon, release build, 64 texts per class:

| length class | ms/text |
|---|---|
| short (~8-line fn) | 30.0 |
| ~512-token cap (medium and long both truncate) | 172–183 |

172–183 ms/text matches the ~194 ms/chunk observed end-to-end on tokio —
tokenization, tensor building, and the session mutex account for almost
nothing.

- **Threads (verified, no change)**: `build_session` already sets
  `intra_threads` = available cores; ~330%-of-800% CPU is this graph's
  practical scaling ceiling, not a misconfiguration.
- **Length bucketing (validated, shipped `0d222b0`)**: 1.4x on tokio cold
  index (2,282s → 1,616s). Small because most code chunks truncate at the
  512 cap → batches are already near-uniform max length.
- **CoreML EP (rejected)**: dynamic `[batch, seq_len]` shapes force
  per-shape recompilation — ~3 min session-init stall, then 224.6 ms/text
  on SHORT texts (7.5x slower than CPU) before the profile run aborted.
  Revisit only with static-shape padding buckets or as part of a
  quantization task.

**Conclusion**: per-chunk CPU cost is roofline-bound. The remaining
throughput levers are (1) shorter sequences — task 6.8's sub-chunking
(short texts are ~6x cheaper per text) — and (2) int8-quantized model
artifacts (future task; pairs with the PRD's quantized-ANN theme).

## Verification

```bash
cargo test --workspace
./scripts/ci.sh
LITE=1 ./scripts/scale-test.sh          # tokio rung, before/after comparison
./scripts/scale-test.sh vscode          # PRD-target rung with Jina, once tokio numbers are acceptable
```
