# Task 0.2 — CI skeleton (GitHub Actions)

**Phase**: 0 — Scaffolding
**Task ID**: 0.2
**PRD reference**: VEKTOR_ROADMAP.md Stage 2 brought-forward C8.1 (lite matrix)
**Roadmap stage**: Stage 2 / pre-`v0.1.0`
**Effort estimate**: M (1–4h)
**Depends on**: 0.1
**Blocks**: 1.7

## Objective

Create the initial GitHub Actions CI workflow that runs on every push to `main` and every PR. The lite matrix covers macOS-aarch64 + linux-x86_64; the full 5-platform matrix lands in task 6.2 with the release pipeline.

The point of this task is to make every subsequent code-landing task auditable by CI. Without this, "the tests pass" is something we say; with this, "the tests pass" is something CI proves.

## Inputs (must exist before starting)

- `Cargo.toml` from task 0.1
- A GitHub repository (the local repo will be pushed eventually; CI lives in `.github/workflows/`)

## Outputs (must exist after completion)

- `.github/workflows/ci.yml` — main CI workflow
- `.github/workflows/README.md` (optional) — one-paragraph explanation of the workflow files

## Approach

1. Create `.github/workflows/ci.yml`. The workflow should:
   - Trigger on `push` to `main` and on `pull_request` targeting `main`
   - Use a matrix: `os: [macos-latest, ubuntu-latest]` (lite matrix; full matrix in 6.2)
   - Cache the Cargo registry, git index, and `target/` via `Swatinem/rust-cache@v2`
   - Install Rust 1.88 via `dtolnay/rust-toolchain@stable` reading from `rust-toolchain.toml`
   - Steps in order: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo doc --workspace --no-deps`
   - Fail fast: any step failure stops the job
2. Use `concurrency` group on `${{ github.workflow }}-${{ github.ref }}` with `cancel-in-progress: true` so old runs are killed when a PR is updated
3. Set timeouts: `timeout-minutes: 30` on each job (first build is slow because of `ort` download; subsequent runs are fast due to cache)
4. Workflow permissions: `contents: read` only (read-only by default; never grant write unless a job needs it)
5. Push to the GitHub repo and verify a real CI run completes green

## Acceptance criteria

- [ ] `.github/workflows/ci.yml` exists and is syntactically valid (`gh workflow view ci.yml` succeeds or `actionlint` passes)
- [ ] On push to main, all matrix jobs run and report success in the Actions tab
- [ ] On a deliberately-broken PR (e.g., add `let x: u32 = "string";` to main.rs), CI fails — verify with a throwaway PR
- [ ] `cargo fmt --check` step catches unformatted code
- [ ] `cargo clippy -D warnings` step blocks any clippy warning
- [ ] Build cache hit rate is >80% on second run (verify in Actions log: "Cache restored from key: ...")
- [ ] Total wall-clock time for a cache-hit run is <5 minutes per matrix job

## Verification

```bash
# Local syntax check
actionlint .github/workflows/ci.yml  # install via `brew install actionlint`

# Push and observe
git push origin main
gh run watch  # waits for the run to finish

# Confirm the most recent run completed successfully.
# `jobs` is NOT a valid field on `gh run list --json` (only on `gh run view`),
# so we do this in two steps: find the run ID, then view its jobs.
RUN_ID=$(gh run list --workflow=ci.yml --limit 1 --json databaseId -q '.[0].databaseId')
[ -n "$RUN_ID" ] || { echo "FAIL: no run found for workflow ci.yml"; exit 1; }

# Top-level conclusion
gh run view "$RUN_ID" --json conclusion -q '.conclusion'   # expect: success

# Matrix job names
gh run view "$RUN_ID" --json jobs -q '.jobs[].name'
```

Expected output: `success` for conclusion, and two job names like `build (macos-latest)` and `build (ubuntu-latest)`.

> **Valid `gh run list --json` fields** (as of gh CLI 2.x): `attempt`, `conclusion`,
> `createdAt`, `databaseId`, `displayTitle`, `event`, `headBranch`, `headSha`, `name`,
> `number`, `startedAt`, `status`, `updatedAt`, `url`, `workflowDatabaseId`, `workflowName`.
> The `jobs` field lives on `gh run view`, not `gh run list`. Verify against your
> installed gh version with `gh run list --json help`.

## Reference workflow content (don't copy blindly — adapt to actual needs)

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

permissions:
  contents: read

jobs:
  build:
    name: build
    runs-on: ${{ matrix.os }}
    timeout-minutes: 30
    strategy:
      fail-fast: false
      matrix:
        os: [macos-latest, ubuntu-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.88.0
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
      - run: cargo doc --workspace --no-deps
```

## Notes / open questions

- **Windows in matrix?** Not in this lite version. The full matrix (including windows-latest + linux-aarch64) lands in task 6.2 with the release pipeline. Adding Windows here would double the first-run download time (ORT for Windows) for marginal value at v0.1.
- **macOS-aarch64**: GitHub provides this as `macos-latest` (M-series). If the GHA runner switches default, this could regress; pin to `macos-14` explicitly if it becomes unstable.
- **Codecov / coverage**: deferred. Coverage tooling adds 5+ minutes to CI; defer until we have meaningful code (Phase 2+). Add as part of task 2.9 (v0.2.0 release).
- **Secrets**: this workflow needs zero secrets at v0.1.0. Token-based PR comment writers and release-artifact uploads come later. Keep the workflow secret-free until task 6.2.
- **First run will be slow** (10–15 min) because of ort download and full Cargo build. Subsequent runs should be ≤5 min with cache.

## Commit

```
ci(workflow): 0.2 — add lite CI matrix (macOS + Linux) with fmt/clippy/test/doc

Trigger on push to main and on PRs. Two-job matrix runs cargo fmt,
clippy with -D warnings, full test suite, and doc build. Swatinem
cache pinned to v2 for fast incremental runs. Concurrency group
cancels in-progress runs on PR updates. Timeouts 30min per job.

Full 5-platform matrix and release artifacts land in task 6.2.

Closes docs/plans/initial/phase-0-scaffolding/02-ci-skeleton.md
```
