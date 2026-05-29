# Task 0.4 — cargo-deny supply-chain baseline

**Phase**: 0 — Scaffolding
**Task ID**: 0.4
**PRD reference**: PRD Section 13 (Risks — dependency-related entries)
**Roadmap stage**: Stage 2 / pre-`v0.1.0`
**Effort estimate**: S (≤1h)
**Depends on**: 0.1
**Blocks**: 1.7a

## Objective

Establish a supply-chain security baseline using [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny). This task runs cargo-deny once against the current dependency tree, captures the output as the **baseline** (existing advisories we accept), and wires cargo-deny into CI in advisory mode.

Strict enforcement (failing CI on new advisories) lands in task 6.2 with the release pipeline. At v0.1.0, the goal is *visibility*, not *gating*.

## Inputs (must exist before starting)

- `Cargo.toml` and `Cargo.lock` from task 0.1
- CI workflow from task 0.2 (cargo-deny gets added as a new step or job)

## Outputs (must exist after completion)

- `/deny.toml` — cargo-deny config defining policy
- `/.github/workflows/audit.yml` (separate workflow file) — runs cargo-deny on a schedule + on PRs that touch `Cargo.toml` or `Cargo.lock`
- Captured baseline noted in `deny.toml` `[advisories.ignore]` for any existing advisories that don't have fixes (rare for fresh deps but possible)

## Approach

1. Install cargo-deny locally: `cargo install cargo-deny`
2. Run `cargo deny init` to generate a starter `deny.toml` **using the current version's
   default schema**. Do NOT hand-write the file from older examples — cargo-deny's config
   format has been versioned (the v2 schema replaced the old `vulnerability = "warn"` /
   `unmaintained = "warn"` / `unsound = "warn"` fields). Treat what `cargo deny init`
   generates as the canonical starting point and modify it. Reference docs:
   https://embarkstudios.github.io/cargo-deny/checks/advisories/cfg.html
3. Review and edit the generated `deny.toml`:
   - `[licenses]`: allow `MIT`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`,
     `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`, `Unicode-DFS-2016`, `MPL-2.0`.
     Deny `GPL-*`, `AGPL-*`, `LGPL-*` (incompatible with MIT distribution).
   - `[bans]`: error on duplicate versions of `arrow*` crates (these indicate a
     lancedb pin issue per PRD Section 11 fix). Allow our explicit deps.
   - `[sources]`: only allow `crates.io` and `github.com` as sources. No tarballs,
     no git deps for now.
   - `[advisories]`: configure severity per the v2 schema. The current cargo-deny
     uses unified handling rather than per-category fields. Whatever
     `cargo deny init` generates is the right starting point.
4. Run `cargo deny check` and inspect output. For each advisory:
   - **Has a fix in a newer version**: update the dep version. Re-run `cargo deny check`.
   - **No fix available**: add the advisory ID to the `ignore = [...]` array inside
     `[advisories]` — this is an array field, NOT a separate `[advisories.ignore]`
     sub-table. Use the structured form so the reason survives:
     ```toml
     [advisories]
     # ... (other v2 fields from cargo deny init)
     ignore = [
         { id = "RUSTSEC-2024-XXXX", reason = "no fix available; transitive dep of X; tracked in <issue link>" },
     ]
     ```
     Reference: https://embarkstudios.github.io/cargo-deny/checks/advisories/cfg.html#the-ignore-field
   - **License mismatch**: investigate. If a transitive dep has a non-allowed license,
     file an issue and decide whether to drop the parent dep.
4. Create `.github/workflows/audit.yml`:
   ```yaml
   name: Audit
   on:
     push:
       paths: [Cargo.toml, Cargo.lock, deny.toml]
     pull_request:
       paths: [Cargo.toml, Cargo.lock, deny.toml]
     schedule:
       - cron: '0 6 * * 1'  # every Monday at 06:00 UTC
   jobs:
     cargo-deny:
       runs-on: ubuntu-latest
       steps:
         - uses: actions/checkout@v4
         - uses: EmbarkStudios/cargo-deny-action@v2
   ```
5. Push and verify the audit workflow runs green (or with only the expected baseline warnings).

## Acceptance criteria

- [ ] `cargo deny check` runs locally and produces a `passing` or `warning` result (no `error`-level findings without an explicit ignore entry)
- [ ] `deny.toml` is committed with the agreed allow-list of licenses
- [ ] `.github/workflows/audit.yml` exists and runs on Cargo.toml changes + weekly schedule
- [ ] First audit workflow run completes green
- [ ] No GPL/AGPL-licensed transitive dependency exists (this would be a real blocker; investigate before merging)
- [ ] `[bans]` section configured to error on duplicate `arrow*` versions (Section 11 fix)

## Verification

```bash
# Local
cargo install cargo-deny
cargo deny check 2>&1 | tee /tmp/deny.log
grep -E "^(error|warning)" /tmp/deny.log
# A clean baseline either has zero output here or only warnings we've explicitly ignored

# CI
gh workflow run audit.yml
gh run watch --workflow audit.yml
```

## Notes / open questions

- **License allowlist is opinionated**: the list above is the conservative MIT-compatible set. If a useful crate has a license not on the list (e.g., MPL-2.0 with file-level copyleft), add it explicitly with a rationale comment in `deny.toml`.
- **Advisory mode at v0.1.0, strict at v0.4.0**: cargo-deny config has been versioned (v2 schema is current as of cargo-deny ~0.16+). The v2 schema unified the older per-category fields (`vulnerability = "warn"` etc.). Use the severity defaults that `cargo deny init` generates at v0.1.0 — this is the "make CI report but not block" stance. Strict promotion (block on new advisories) lands in task 6.2 with the release pipeline. Source of truth for current config schema: https://embarkstudios.github.io/cargo-deny/checks/advisories/cfg.html — verify before writing `deny.toml`.
- **Duplicate version ban for `arrow*`**: PRD Section 11 explicitly warns about Arrow version coupling with lancedb. cargo-deny's `[bans]` is the enforcement mechanism. If task 0.1's `cargo tree -d` showed any duplicates, those must be resolved before this task can pass.
- **EmbarkStudios cargo-deny is mature**: it's been maintained since 2019, used by tokio, Bevy, and many others. Safe to depend on.

## Commit

```
chore(deny): 0.4 — cargo-deny baseline + audit workflow

Adds cargo-deny config with a conservative MIT-compatible license
allowlist, ban on duplicate arrow* versions (per PRD Section 11
Arrow coupling fix), and crates.io+github.com as the only allowed
sources. Advisories are in warn-mode at v0.1.0; promoted to deny
in task 6.2 with the release pipeline.

Separate audit workflow runs on Cargo.toml/lock/deny.toml changes
plus a weekly schedule.

Closes docs/plans/initial/phase-0-scaffolding/04-cargo-deny.md
```
