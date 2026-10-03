# Task 6.4 — `BENCHMARKS.md` baseline

## Objective

Commit the initial v0.4.0 benchmark baseline and wire CI to surface benchmark deltas on pull requests.

## Scope

- Add `BENCHMARKS.md` with the initial tokio 20-query baseline.
- Record benchmark environment, commit SHA, model/backend, and index configuration.
- Add CI output that posts or uploads benchmark deltas.
- Keep deltas informational for v0.4.0.

## Acceptance Criteria

- [x] `BENCHMARKS.md` contains baseline metrics and reproduction commands.
- [x] CI captures benchmark output.
- [x] PR benchmark deltas are visible to reviewers.
- [x] Benchmark metadata includes enough context to reproduce results.

## Verification

```bash
cargo bench
test -f BENCHMARKS.md
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```
