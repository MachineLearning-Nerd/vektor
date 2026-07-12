# Task 6.3 — Lightweight benchmark gate

## Objective

Add a 20-query labeled benchmark against tokio source so Phase 5 retrieval quality has a repeatable launch baseline.

## Scope

- Create a compact labeled query fixture.
- Add a benchmark runner that indexes the checked-in tokio fixture and evaluates Precision@5, Recall@5, and MRR.
- Keep the benchmark report-only for v0.4.0.
- Avoid requiring network during normal CI unless fixtures are explicitly prepared.

## Acceptance Criteria

- [x] 20 labeled queries are checked in.
- [x] Benchmark runner emits Precision@5, Recall@5, and MRR.
- [x] Benchmark can run locally with `cargo bench --bench retrieval_quality`.
- [x] Benchmark is report-only for v0.4.0 and avoids network by using checked-in fixtures.

## Verification

```bash
cargo bench
cargo test benchmark
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```
