# Phase 1 — Skeleton (`v0.1.0`)

> **Goal**: `vektor index` and `vektor serve` parse CLI args; `vektor serve` registers a no-op MCP server with rmcp 1.7. README and LICENSE are in place. CI is green on macOS + Linux. `v0.1.0` is a notes-only release: source install from the Git tag, no binary attachments.

**Roadmap mapping**: Stage 2 / `v0.1.0`
**PRD mapping**: Section 12 Week 1 — **Function 1.1 only** (binary entry point). PRD Functions 1.2 (`discover_files`), 1.3 (`hash_file`), 1.4 (`HashStore`) are intentionally deferred to Phase 2 (tasks 2.1 and 2.2) because v0.1.0 ships only the CLI/MCP skeleton — no real indexing yet. Plus brought-forward launch prereqs (CI matrix lite, README, LICENSE — already shipped in Phase 0).
**Effort estimate**: ~3 weeks of focused part-time work (M/L per task; release polish takes the longest)
**Status**: 🟡 Implementation + release prep complete in working tree; `1.7b` publish is pending explicit release action

---

## Gated Execution Flow

Each implementation task is treated as complete only after:

1. The task implementation is finished.
2. The task's verification commands and the Phase 1 baseline pass:
   - `cargo fmt --check`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo test --workspace`
   - `git diff --check`
3. A task-scoped review pass checks the diff and acceptance criteria.
4. Valid review comments are fixed, verification is rerun, and review repeats up to 5 rounds.
5. If valid blocker comments remain after 5 rounds, phase execution stops.

After `1.7a` is clean, run one combined Phase 1 review before `1.7b`. `1.7b` is deliberately separated because push/tag/repo-visibility/release actions are hard to undo cleanly.

---

## Task List

| ID | Task | Effort | Depends on | Status | Commit |
|---|---|---|---|---|---|
| 1.1 | [main entrypoint](01-main-entrypoint.md) | S | 0.1, 0.3 | ✅ | `16a8c77` |
| 1.2 | [error module — thiserror](02-error-module.md) | S | 1.1 | ✅ | `16a8c77` |
| 1.3 | [config module — TOML + env override](03-config-module.md) | M | 1.2 | ✅ | `16a8c77` |
| 1.4 | [CLI args — clap derive subcommands](04-cli-args.md) | M | 1.1, 1.3 | ✅ | `16a8c77` |
| 1.5 | [tracing init](05-tracing-init.md) | S | 1.4 | ✅ | `16a8c77` |
| 1.6 | [MCP no-op handlers — rmcp 1.7](06-mcp-noop-handlers.md) | L | 1.5 | ✅ | `16a8c77` |
| 1.7a | [release prep docs + notes](07-v0.1.0-release.md) | S | 0.2, 0.4, 1.6 | ✅ | `16a8c77` |
| 1.7b | [publish v0.1.0 tag + GitHub Release](08-v0.1.0-publish.md) | S | 1.7a + combined review | ⬜ | — |

---

## Phase Exit Criteria

Implementation gates:

- [x] `vektor --help` shows the 3 planned subcommands: `index`, `serve`, `models`
- [x] `vektor serve` starts an MCP server over stdio, accepts `initialize`, `tools/list`, and `tools/call`, and returns the planned tool names (handlers are no-op)
- [x] `vektor index /some/path` parses arguments and exits with a "not implemented" message
- [x] `vektor models download` parses arguments and exits with a "not implemented" message
- [x] Config override tests are deterministic and isolate `HOME`/`USERPROFILE`
- [x] Tracing logs go to stderr and do not contaminate MCP stdout
- [x] `README.md` documents source install with `cargo install --locked` and the `protoc` prerequisite
- [x] `release-notes-v0.1.0.md` exists and is notes-only
- [x] `docs/plans/initial/phase-2-discovery-chunking/` has per-task files for all 9 Phase 2 tasks

Publish gates owned by `1.7b`:

- [ ] Final release commit pushed to `origin/main`
- [ ] Latest `ci.yml` run for the release commit explicitly concludes `success`
- [ ] Repository visibility is public before the unauthenticated `cargo install --git ... --tag v0.1.0 --locked` gate
- [ ] Git tag `v0.1.0` exists and is pushed to `origin`
- [ ] GitHub Release `v0.1.0` exists with zero binary attachments
- [ ] `cargo install --git https://github.com/MachineLearning-Nerd/vektor --tag v0.1.0 --locked` succeeds on a fresh unauthenticated machine with `protoc` installed

---

## What v0.1.0 Intentionally Does Not Include

- Any actual indexing (Phase 2)
- Any actual embedding (Phase 3)
- Any real search (Phase 4)
- Context assembly (Phase 5)
- `vektor init` MCP config writer (Phase 6 — task 6.1)
- Release pipeline beyond a single hand-tagged notes-only release (Phase 6 — task 6.2)

---

## Notes

- **The MCP server is a no-op**: every tool handler returns a `{ "status": "not implemented yet", "phase": "<which-phase>" }` JSON object. This proves the rmcp wiring works without requiring real indexing/search logic.
- **Tool list at v0.1.0**: `index_codebase`, `search_code`, `get_context_for_prompt` (the 3 primary tools per Roadmap Stage 2 staging).
- **Commit hashes**: Phase 1 tasks 1.1–1.7a all landed in the single Phase 1 commit `16a8c77`. Per-task commits weren't reconstructable from the final tree (later tasks rewrote earlier tasks' code — e.g. the dead-code placeholder added in 1.2/1.3 was removed in 1.6), so a milestone commit is the honest representation. Task 1.7b is a publish action tracked by the `v0.1.0` tag + GitHub Release, not a code commit.

---

## When This Phase Completes

1. Run combined Phase 1 review and full verification.
2. Commit the release-prep working tree.
3. Push the final release commit and wait for `ci.yml` success on that exact commit.
4. Make the GitHub repo public if still private.
5. Tag `v0.1.0`, push the tag, and create the notes-only GitHub Release.
