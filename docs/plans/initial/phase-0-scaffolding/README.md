# Phase 0 — Scaffolding

> **Goal**: `git clone && cargo check` succeeds. No application code yet, but the build pipeline, CI, and pre-commit hooks all work. **No release tag** from this phase.

**Roadmap mapping**: prerequisite for Stage 2 / v0.1.0
**Effort estimate**: 4–8 hours total (all S/M tasks)
**Status**: ✅ **Done** — all 4 tasks complete; Phase 1 unblocked.

---

## Task list

| ID | Task | Effort | Status | Commit |
|---|---|---|---|---|
| 0.1 | [cargo init + Cargo.toml](01-cargo-init.md) | S | ✅ Done | `a3a2c2a` (preceded by spec fixes `93bc389`, `a5d22fd`) |
| 0.2 | [CI skeleton — GitHub Actions](02-ci-skeleton.md) | M | ✅ Done | `73ed9d9` (followed by protoc fix `080305d`) |
| 0.3 | [Pre-commit hooks — rustfmt + clippy](03-pre-commit-hooks.md) | S | ✅ Done | `689d5b2` |
| 0.4 | [cargo-deny supply-chain baseline](04-cargo-deny.md) | S | ✅ Done | `08361cc` |

---

## Phase exit criteria

All must be true before declaring Phase 0 done:

- [ ] `cargo check` runs cleanly with the resolved Section 11 dependencies
- [ ] GitHub Actions runs on every push to `main`: `cargo fmt --check` + `cargo clippy -- -D warnings` + `cargo test`
- [ ] Pre-commit hook blocks commits where `cargo fmt --check` or `cargo clippy` fails
- [ ] `cargo deny check` runs in CI and produces a non-failing baseline (existing advisories tracked in `deny.toml`)
- [ ] Phase 1's first task (1.1 main entrypoint) can start without blockers

---

## Notes

- This phase does **not** ship any user-visible feature. It's the foundation for everything else.
- Pre-commit hooks are local-only (developer machines); CI is the source of truth for "did checks pass."
- `cargo-deny` is run in *advisory mode* at this stage — we don't block PRs on it yet. Strict enforcement comes in Phase 6 with the release pipeline.

---

## When this phase completes

1. Mark all tasks ✅ in the table above with commit hashes
2. Update `docs/plans/initial/README.md` Current State
3. Phase 1 (1.1) is now ready to start
