# Phase 6 — Launch Polish (`v0.4.0`)

> **Goal**: `vektor init` writes MCP config for Claude Code/Cursor/Codex on first run. A signed release pipeline is ready to produce signed binaries on tag push for the full 5-platform matrix. A 20-query labeled benchmark writes the baseline to `BENCHMARKS.md`. The local-first batch prepares `v0.4.0`; tag, release publication, crates.io, and public visibility changes require separate authorization.

**Roadmap mapping**: Stage 2 / `v0.4.0` (the launchable alpha)
**PRD mapping**: Section 7.7 (`vektor init`), Section 14 Phase 1 Must-Have criteria, plus brought-forward C8.1 (full matrix) + C8.2 (signed binaries) + BM1 (lightweight benchmark)
**Effort estimate**: 2–3 weeks of focused part-time work
**Status**: ✅ Local-first implementation complete and verified — PR review pending; release/public actions remain deferred

---

## Task list

| ID | Task | Effort | Depends on | Status |
|---|---|---|---|---|
| 6.1 | `vektor init` MCP-config writer — detect Claude Code, Cursor, Codex CLI; write entries; `--force`, `--agent`, `--dry-run` flags (per PRD §7.7) | M | 4.7 | ✅ Done — `(this commit)` |
| 6.2 | Release pipeline — extend `ci.yml` matrix to full 5 platforms (macOS-x86_64/aarch64, linux-x86_64/aarch64, windows-x86_64); separate `release.yml` triggered by tag push; signed binaries + SHA-256 + SBOM | L | 0.2, 1.7b (start after v0.1 publish; finalize here) | ✅ Done — local scaffolded; live release run deferred |
| 6.3 | Lightweight benchmark gate — 20 hand-labeled queries against tokio's source, run via `cargo bench`, output to `BENCHMARKS.md` | M | 5.13 | ✅ Done — checked-in tokio fixture + report-only runner |
| 6.4 | `BENCHMARKS.md` baseline — initial commit + CI step that posts benchmark deltas on PRs | S | 6.3 | ✅ Done — baseline + informational PR delta workflow |
| 6.5 | `v0.4.0` release readiness docs + authorization checklist | M | 5.13, 6.1, 6.2, 6.4 | ✅ Done — docs/checklist only; tag/publish deferred |
| 6.7 | Embedding throughput investigation — profile the ~194 ms/chunk cold-index embed cost (ONNX threads, batch size) found by the scale-test harness; measure-first | M | 6.3 | ⬜ Not started — not a v0.4.0 blocker |
| 6.8 | Chunk size vs embedder window alignment — 200-line chunks truncate at bge-small's 512 tokens, so lite embeddings are head-only; decide sub-chunk / accept / window-pool with benchmark evidence | M | 6.3, 6.7 | ⬜ Not started — not a v0.4.0 blocker |

---

## Local-first exit criteria

All must be true before opening the Phase 6 PR:

- [x] All 5 local-first tasks marked ✅ Done
- [x] `vektor init` tests cover Claude Code (`~/.claude.json`), Cursor (`~/.cursor/mcp.json`), and Codex (`~/.codex/config.toml`) config writes; preserves existing entries; refuses overwrite without `--force`
- [x] Release workflow is scaffolded for all 5 platforms
- [x] Release workflow generates checksum, signature, certificate, SBOM metadata, and provenance artifacts
- [x] `BENCHMARKS.md` has the `v0.4.0` baseline showing Precision@5 / Recall@5 / MRR on the 20 Tokio fixture queries
- [x] Release notes and release authorization checklist are drafted
- [x] Final local gate passes on the branch before push/PR

## Deferred release authorization gates

These are not part of the local-first implementation batch:

- [ ] Final crate/package name decision before any crates.io publish
- [ ] `v0.4.0` tag creation
- [ ] GitHub Release publication and artifact upload
- [ ] Tag-triggered release workflow execution
- [ ] Cross-platform binary smoke tests
- [ ] Repository visibility flip / `6.6 go public`

---

## Notes

- **`vektor init` complexity**: each agent has its own config schema. Claude Code uses `~/.claude.json` with a specific `mcpServers` object. Cursor uses `~/.cursor/mcp.json`. Codex CLI uses `~/.codex/config.toml`. The init command must read each, merge entries idempotently, and never clobber unrelated config.
- **Signing strategy**: at v0.4.0, simple Sigstore via [`cosign`](https://github.com/sigstore/cosign) is sufficient. Full SBOM generation (CycloneDX) ships at task C8.6 in Stage 6 — we don't need it at v0.4.0.
- **Benchmark labeling**: this is the 20-query subset from C1 (full corpus is Stage 3 / 150 queries). For v0.4.0, picking 20 queries against tokio gives a representative baseline without requiring 2 weeks of labeling work. Lift to 50/repo and add 3 repos in Stage 3.
- **Per-PR benchmark gate**: at v0.4.0, the benchmark just reports — it doesn't block PRs. Strict regression-gating is a Stage 6 (C8.4) feature once we have multiple data points to calibrate "what is a real regression."
- **Crate-name gate**: the deferred decision (`vektor` is taken on crates.io) is documented in `release-checklist-v0.4.0.md`. It blocks crates.io publish only; it does not block the local-first PR.
- **Scale-test follow-ups (6.7/6.8)**: added 2026-07-12 after `scripts/scale-test.sh` runs against real repos exposed an untruncated-sequence embedding crash (fixed, `0d222b0`) and Lance version churn (fixed, `e94ca54`). 6.7 (throughput) and 6.8 (chunk/window quality) are the remaining items; 6.6 stays reserved for the go-public action. Neither blocks `v0.4.0` under the current exit criteria, but 6.8 affects lite-model retrieval quality — reconsider scope if lite is the recommended first-run path at launch.

---

## When this phase completes

1. Final local gate passes
2. Push `feature/phase-6-launch-polish`
3. Open one Phase 6 PR
4. Review and fix PR comments
5. After merge and explicit release authorization, execute `release-checklist-v0.4.0.md`
6. After public launch authorization, execute `6.6 go public`
7. **Begin planning Stage 3** — write `docs/plans/stage-3-workflow-tools/` with the same structure as this directory
