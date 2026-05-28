# Phase 1 — Skeleton (`v0.1.0`)

> **Goal**: `vektor index` and `vektor serve` parse CLI args; `vektor serve` registers a no-op MCP server with rmcp 1.7. README and LICENSE in place. CI green on macOS + Linux. **Tagged `v0.1.0` and installable via `cargo install --git ... --tag v0.1.0`** — release is notes-only at v0.1.0 (binary attachments are scoped to v0.4.0 / task 6.2 per the roadmap).

**Roadmap mapping**: Stage 2 / `v0.1.0`
**PRD mapping**: Section 12 Week 1 — **Function 1.1 only** (binary entry point). PRD Functions 1.2 (`discover_files`), 1.3 (`hash_file`), 1.4 (`HashStore`) are intentionally deferred to Phase 2 (tasks 2.1 and 2.2) because v0.1.0 ships only the CLI/MCP skeleton — no real indexing yet. Plus brought-forward launch prereqs (CI matrix lite, README, LICENSE — already shipped in Phase 0).
**Effort estimate**: ~3 weeks of focused part-time work (M/L per task; release polish takes the longest)
**Status**: ⬜ Not started

---

## Task list

| ID | Task | Effort | Depends on | Status | Commit |
|---|---|---|---|---|---|
| 1.1 | [main entrypoint](01-main-entrypoint.md) | S | 0.1, 0.3 | ⬜ | — |
| 1.2 | [error module — thiserror](02-error-module.md) | S | 1.1 | ⬜ | — |
| 1.3 | [config module — TOML + env override](03-config-module.md) | M | 1.2 | ⬜ | — |
| 1.4 | [CLI args — clap derive subcommands](04-cli-args.md) | M | 1.1, 1.3 | ⬜ | — |
| 1.5 | [tracing init](05-tracing-init.md) | S | 1.4 | ⬜ | — |
| 1.6 | [MCP no-op handlers — rmcp 1.7](06-mcp-noop-handlers.md) | L | 1.5 | ⬜ | — |
| 1.7 | [v0.1.0 release tag + notes](07-v0.1.0-release.md) | M | 0.2, 0.4, 1.6 | ⬜ | — |

---

## Phase exit criteria

All must be true before tagging `v0.1.0`:

- [ ] All 7 tasks above marked ✅ Done with commit hashes recorded
- [ ] `vektor --help` shows the 3 planned subcommands: `index`, `serve`, `models`
- [ ] `vektor serve` starts an MCP server over stdio, accepts a `tools/list` request, and returns the planned tool names (handlers are no-op — they return `{ "status": "not implemented yet" }`)
- [ ] `vektor index /some/path` parses arguments and exits with a "not implemented" message (does not crash, does not silently succeed)
- [ ] `vektor models download` parses arguments and exits with a "not implemented" message
- [ ] CI green on macOS-latest + ubuntu-latest with `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test`, `cargo doc`
- [ ] `gh release create v0.1.0` creates a notes-only GitHub Release (zero binary attachments at v0.1.0 per the roadmap; binaries land at v0.4.0 / task 6.2)
- [ ] `cargo install --git https://github.com/MachineLearning-Nerd/vektor --tag v0.1.0` succeeds on a fresh machine
- [ ] `README.md` walks a fresh user from `cargo install --git ... --tag v0.1.0` (the source-install path; v0.1.0 ships no binary attachments per task 1.7 / roadmap) through to `vektor --help` in 3 commands, with the `protoc` prerequisite called out before the install command

---

## What v0.1.0 *intentionally does not* include

Listed here so we don't drift into scope creep:

- Any actual indexing (Phase 2)
- Any actual embedding (Phase 3)
- Any real search (Phase 4)
- Context assembly (Phase 5)
- `vektor init` MCP config writer (Phase 6 — task 6.1)
- Release pipeline beyond a single hand-tagged release (Phase 6 — task 6.2)

If during Phase 1 execution you find yourself implementing any of the above, **stop**. Move the work to its proper phase and leave a "Not Implemented Yet" stub here.

---

## Notes

- **The MCP server is a no-op**: every tool handler returns a `{ "status": "not implemented yet", "phase": "<which-phase>" }` JSON object. This proves the rmcp wiring works without requiring any real logic. Agents calling Vektor v0.1.0 will get useless answers, but the protocol round-trips correctly.
- **Tool list at v0.1.0**: `index_codebase`, `search_code`, `get_context_for_prompt` (the 3 primary tools per Roadmap Stage 2 staging). The 5 workflow tools (`get_context_for_task` etc.) don't exist yet — they ship in Stage 3.
- **README.md and LICENSE are already in the repo** (committed in `3d81fa6`). Tasks 1.1–1.7 don't need to create them, but task 1.7 should verify they're current and the README matches what v0.1.0 actually does.

---

## When this phase completes

The sequence is owned by task 1.7 — follow its Approach steps in order. Summary for quick reference:

1. Pre-flight: CI green on main, tasks 0.1–1.6 all ✅, working tree clean (per task 1.7 step 1)
2. Generate `release-notes-v0.1.0.md` from commit history (per task 1.7 step 3)
3. Expand `phase-2-discovery-chunking/` per-task files (per task 1.7 step 4)
4. Update tracker docs (this README, `plans/initial/README.md`, `DEPENDENCIES.md`, `VEKTOR_ROADMAP.md`) (per task 1.7 step 5)
5. Final commit + push + **wait for CI green on the released commit** (per task 1.7 step 6)
6. Tag `v0.1.0` and push the tag (per task 1.7 step 7)
7. Create the GitHub Release with `--notes-file release-notes-v0.1.0.md` — no binary attachments (per task 1.7 step 9)

Do NOT shortcut this sequence — task 1.7's reviewer-validated ordering exists to keep the tag pointing at a CI-validated commit.
