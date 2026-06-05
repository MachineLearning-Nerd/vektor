# Task 6.3 — Lightweight benchmark gate

## Objective

Add a 20-query labeled benchmark against tokio source so Phase 5 retrieval quality has a repeatable launch baseline.

## Scope

- Create a compact labeled query fixture.
- Add a benchmark runner that indexes tokio and evaluates Precision@5, Recall@5, and MRR.
- Keep the benchmark report-only for v0.4.0.
- Avoid requiring network during normal CI unless fixtures are explicitly prepared.

## Acceptance Criteria

- [ ] 20 labeled queries are checked in.
- [ ] Benchmark runner emits Precision@5, Recall@5, and MRR.
- [ ] Benchmark can run locally with documented setup.
- [ ] CI can run or report the benchmark without blocking PRs.

## Verification

```bash
cargo bench
cargo test benchmark
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```
