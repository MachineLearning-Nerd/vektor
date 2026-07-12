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

- [ ] Per-stage timing profile of the embed path is captured and documented (where do the 194 ms/chunk actually go?).
- [ ] ONNX intra-op thread configuration is verified against the host core count, with measured impact.
- [ ] At least one lever is validated or explicitly rejected with before/after `LITE=1 scripts/scale-test.sh` numbers (`failed: 0` on both sides).
- [ ] Peak RSS does not regress above the current ~2.5 GB, and the chosen config is justified against the PRD <700 MB target (or the target's revision is proposed).
- [ ] Embedding output is unchanged for identical input (vectors are batching-invariant modulo float noise).

## Verification

```bash
cargo test --workspace
./scripts/ci.sh
LITE=1 ./scripts/scale-test.sh          # tokio rung, before/after comparison
./scripts/scale-test.sh vscode          # PRD-target rung with Jina, once tokio numbers are acceptable
```
