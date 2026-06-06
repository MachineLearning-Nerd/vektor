# Phase 6 — Launch Polish (`v0.4.0`)

> **Goal**: `vektor init` writes MCP config for Claude Code/Cursor/Codex on first run. A signed release pipeline produces signed binaries on tag push for the full 5-platform matrix. A 20-query labeled benchmark runs against tokio and writes the baseline to `BENCHMARKS.md`. Tag `v0.4.0` as the first alpha release someone other than the author can install and use.

**Roadmap mapping**: Stage 2 / `v0.4.0` (the launchable alpha)
**PRD mapping**: Section 7.7 (`vektor init`), Section 14 Phase 1 Must-Have criteria, plus brought-forward C8.1 (full matrix) + C8.2 (signed binaries) + BM1 (lightweight benchmark)
**Effort estimate**: 2–3 weeks of focused part-time work
**Status**: ⬜ Not started — per-task files written, implementation not started

---

## Task list

| ID | Task | Effort | Depends on | Status |
|---|---|---|---|---|
| 6.1 | `vektor init` MCP-config writer — detect Claude Code, Cursor, Codex CLI; write entries; `--force`, `--agent`, `--dry-run` flags (per PRD §7.7) | M | 4.7 | ✅ Done — `(this commit)` |
| 6.2 | Release pipeline — extend `ci.yml` matrix to full 5 platforms (macOS-x86_64/aarch64, linux-x86_64/aarch64, windows-x86_64); separate `release.yml` triggered by tag push; signed binaries + SHA-256 + SBOM | L | 0.2, 1.7b (start after v0.1 publish; finalize here) | ⬜ |
| 6.3 | Lightweight benchmark gate — 20 hand-labeled queries against tokio's source, run via `cargo bench`, output to `BENCHMARKS.md` | M | 5.13 | ⬜ |
| 6.4 | `BENCHMARKS.md` baseline — initial commit + CI step that posts benchmark deltas on PRs | S | 6.3 | ⬜ |
| 6.5 | `v0.4.0` release tag + announcement | M | 5.13, 6.1, 6.2, 6.4 | ⬜ |

---

## Phase exit criteria

All must be true before tagging `v0.4.0`:

- [ ] All 5 tasks marked ✅ Done
- [ ] `vektor init` correctly writes MCP entry for at least Claude Code (`~/.claude.json`) and Cursor (`~/.cursor/mcp.json`) on a fresh macOS install; preserves existing entries; refuses to overwrite an existing `vektor` entry without `--force`
- [ ] Tag-triggered release workflow runs successfully for 5 platforms
- [ ] Each release artifact is signed (Sigstore-style) with a verifiable provenance line
- [ ] SHA-256 files match the artifacts byte-for-byte
- [ ] `vektor models download` works on a fresh install of the released binary on each of the 5 platforms (smoke test)
- [ ] `BENCHMARKS.md` has the `v0.4.0` baseline showing Precision@5 / Recall@5 / MRR on the 20 tokio queries
- [ ] The README's "Install" section works end-to-end — a stranger can install from binary, run `vektor init`, register with Claude Code, and call `tools/list` to confirm it's wired up

---

## Notes

- **`vektor init` complexity**: each agent has its own config schema. Claude Code uses `~/.claude.json` with a specific `mcpServers` object. Cursor uses `~/.cursor/mcp.json`. Codex CLI uses `~/.codex/config.toml`. The init command must read each, merge entries idempotently, and never clobber unrelated config.
- **Signing strategy**: at v0.4.0, simple Sigstore via [`cosign`](https://github.com/sigstore/cosign) is sufficient. Full SBOM generation (CycloneDX) ships at task C8.6 in Stage 6 — we don't need it at v0.4.0.
- **Benchmark labeling**: this is the 20-query subset from C1 (full corpus is Stage 3 / 150 queries). For v0.4.0, picking 20 queries against tokio gives a representative baseline without requiring 2 weeks of labeling work. Lift to 50/repo and add 3 repos in Stage 3.
- **Per-PR benchmark gate**: at v0.4.0, the benchmark just reports — it doesn't block PRs. Strict regression-gating is a Stage 6 (C8.4) feature once we have multiple data points to calibrate "what is a real regression."
- **Crate-name rename happens here**: this is the first `cargo publish`. The deferred decision (`vektor` is taken on crates.io) must be made before this task. Update PRD/roadmap/all docs.

---

## When this phase completes

1. Mark all tasks ✅
2. Tag `v0.4.0`
3. Publish GitHub Release with full 5-platform binaries
4. Run a smoke test on each platform's binary
5. Update Current State tables — Stage 2 = ✅ Done
6. Announce `v0.4.0` (blog post, Reddit r/rust, HN, dev.to — whatever distribution channels make sense)
7. **Begin planning Stage 3** — write `docs/plans/stage-3-workflow-tools/` with the same structure as this directory. Phase 1.5 / workflow tools work begins.
