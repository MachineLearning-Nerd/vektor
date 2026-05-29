# Vektor — Implementation Roadmap

> Companion to `VEKTOR_PRD.md`. Tracks **when** we build what.
> The PRD says *what* and *why*. This document says *what's next, why now, and how we know we're done*.

---

## Document Info

| Field | Value |
|---|---|
| Document | VEKTOR_ROADMAP.md |
| Version | 0.1.0 |
| Last Updated | 2026-05-28 |
| Status | Active — Stage 2 in progress |
| Tracks PRD | VEKTOR_PRD.md v2.5.0 |
| Owner | Dinesh (solo) |
| Versioning Convention | Hybrid: Phase = engineering scope, `vX.Y.Z` = cargo release tag |

---

## Versioning Convention

**Phases** describe engineering scope. **Cargo release tags** are the user-visible artifact.
This document uses both:

| Roadmap stage | Cargo release range | What it means |
|---|---|---|
| Stage 1 — PRD Hygiene | (no release) | Documentation only |
| Stage 2 — Core engine | `v0.1.0` → `v0.4.x` | Index + search + 3 primary tools. Pre-alpha; expect breaking changes. |
| Stage 3 — Workflow tools + benchmark | `v0.5.0` → `v0.9.x` | All 8 MCP tools work + labeled corpus exists. Alpha. |
| Stage 4 — v1.0 hardening | `v1.0.0` | Security, threat model, crash-recovery test suite, **plus real-time watcher and dependency graph** (1.0-blockers per review). First "recommended for daily use" release. |
| Stage 5 — Trust layer | `v1.1.0` → `v1.x` | Calibrated confidence, active feedback loop, tool composition, **git history indexing, Ollama backend**. |
| Stage 6 — Platform | `v2.0.0` → `v2.x` | **Extended language support (Java, C/C++, Ruby…), LSP precision layer**, streaming, cost transparency, schema versioning, CI/CD matrix, plugin model, telemetry. |

> **Note on phase vs stage**: The PRD uses "Phase 1/2/3" to describe engineering buckets (what gets built in which order). The roadmap uses "Stage 1–6" to describe release timeline. The mapping is many-to-many — PRD Phase 1 spans roadmap Stages 2 and 3; PRD Phase 2 splits between Stages 4 (watcher + dep graph) and 5 (git history); PRD Phase 3 splits between Stages 3 (benchmarks/C1) and 6 (LSP + extended langs). Always state which you mean.

> **Known issue — crate name `vektor` is taken on crates.io** (`vektor = "0.2.2"`, a SIMD crate). Deferred per current decision; must be resolved before any crates.io publish. The `v0.1.0` milestone is a Git tag + GitHub Release notes milestone and uses `cargo install --git ... --tag v0.1.0`, so it is not blocked by the crate-name decision. Candidates: `vektorctx`, `vektor-mcp`, `kontxt`, or a clean rename. All docs continue to use `vektor` until renamed.

**Semver discipline**: while in `v0.x`, any minor bump may break. From `v1.0` onward, follow strict semver — breaking changes only on major bumps.

---

## Timeline at a Glance

```
Stage 1   Stage 2 (Phase 1 + B6)   Stage 3 (Phase 1.5 + C1)   Stage 4 (v1.0)   Stage 5 (v1.x)   Stage 6 (v2.x)
  ──    ──────────────────────  ──────────────────────────  ──────────────  ──────────────  ──────────────
A1-A3      Core engine               Workflow tools           Hardening      Trust          Platform
PRD edits  + bundled model           + labeled corpus         B1, B2, B5    C2, C3, C4     C5, C6, C7, C8,
                                                                                            C9, C10
                                     ↓                        ↓              ↓               ↓
done →    v0.1 → v0.4              v0.5 → v0.9               v1.0          v1.x            v2.x
```

Each arrow is a hard gate — the next stage does not begin until the previous stage's exit criteria are all met.

---

## Current State (2026-05-28)

| Item | Status |
|---|---|
| PRD v2.5 design complete | ✅ Done |
| A1 — Section 11 version sweep | ✅ Done (commit `025e522`) |
| A2 — Reconcile embedding-backend description | ✅ Done (commit `025e522`) |
| A3 — Fix Sec 14 vs Sec 10 metric conflict | ✅ Done (commit `025e522`) |
| A4 — Expand `.gitignore` | ✅ Done (commit `025e522`) |
| External review addressed (R1–R6: phase rename, scope reconcile, real-time soften, G3 fix, current-state refresh, crate-name note) | ✅ Done (commit `78ddf9a`) |
| Re-sequencing (B1.2/B1.5 → Stage 2, C8.1/C8.2 → Stage 2, `vektor init` added, lightweight benchmark gate at v0.4) | ✅ Done (commit `3d81fa6`) |
| `README.md` exists | ✅ Done (commit `3d81fa6`) |
| `LICENSE` exists | ✅ Done (commit `3d81fa6`) |
| Crate name decision (`vektor` taken on crates.io) | ⏸ Deferred until `cargo publish` |
| `Cargo.toml` exists in repo | ✅ Done (commit `a3a2c2a`) |
| `Cargo.lock` + `src/main.rs` + `rust-toolchain.toml` exist | ✅ Done (commit `a3a2c2a`) |
| Pre-commit hooks configured (task 0.3) | ✅ Done (commit `689d5b2`) |
| cargo-deny baseline + audit workflow (task 0.4) | ✅ Done (commit `08361cc`) |
| GitHub Actions CI workflow (task 0.2) | ✅ Done (commits `73ed9d9` + protoc fix `080305d`) — green on macOS+Linux |
| GitHub repo created and pushed | ✅ Private at https://github.com/MachineLearning-Nerd/vektor |
| **Phase 0 (Scaffolding)** | ✅ **DONE — all 4 tasks complete** |
| Phase 1 skeleton implementation | ✅ Done in working tree (pending commit/tag): async entrypoint, error/config modules, clap CLI, tracing, rmcp stdio no-op handlers, tests |
| Phase 1 release prep (`1.7a`) | ✅ Done in working tree: README install prerequisites, draft release notes, Phase 2 task files, tracker updates |
| `v0.1.0` publish (`1.7b`) | ⬜ Pending explicit release action: push final commit, wait for `ci.yml`, make repo public if required, tag, GitHub Release |

---

## Stage 1 — PRD Hygiene (a single afternoon)

**Release tag**: none (docs only)
**PRD reference**: A1–A4 in the PRD review + external reviewer feedback
**Depends on**: nothing
**Effort**: ~5 hours total (3h hygiene + 2h responding to external review)
**Status**: ✅ Done

### Goal
Bring the PRD into a state where every implementation conversation downstream produces correct code, not stale-version code.

### Deliverables

| # | Item | Status | Effort |
|---|---|---|---|
| A1 | Section 11 `Cargo.toml` re-resolved against current crates.io + 2 version-related risk-table rows updated | ✅ Done | 1h |
| A2 | Section 6.1 / Section 11 / Risk-table embedding-backend descriptions reconciled to one canonical paragraph | ✅ Done | 30 min |
| A3 | Section 14 "Phase 1 Must Have" metric `"index 5K files in <90s"` reconciled with Section 10 revised target | ✅ Done | 15 min |
| A4 | `.gitignore` extended (target/, *.lance, *.onnx, *.db, .DS_Store, ~/.vektor/) | ✅ Done | 5 min |
| R1 | External review fix: rename roadmap Phase 2/3 columns to Stages 5/6 + add many-to-many phase↔stage mapping note | ✅ Done | 20 min |
| R2 | External review fix: PRD Section 8 + Section 14 reconciled to "v0.4 ships 3 tools, v0.5–v0.9 adds 5 workflow tools" | ✅ Done | 15 min |
| R3 | External review fix: Executive Summary real-time claim softened to "planned for v1.0" | ✅ Done | 5 min |
| R4 | External review fix: Goal G3 corrected from "20+ languages" to "5 core, extensible to 20+ in Stage 6" | ✅ Done | 5 min |
| R5 | External review fix: roadmap Current State refreshed (A2/A3/A4 marked done) | ✅ Done | 5 min |
| R6 | External review fix: crate-name conflict noted as deferred decision (`vektor` is taken) | ✅ Done | 5 min |

### Exit criteria (must all be true)

- [ ] Section 11 lists versions resolved by `cargo new && cargo add` in the last 30 days
- [ ] No section contradicts another on embedding-backend choice (`ort` vs `fastembed`)
- [ ] No Phase 1 success metric is unreachable given the revised performance targets
- [ ] `.gitignore` covers all build/state artifacts Phase 1 will produce

### Open decisions
- None

---

## Stage 2 — Core Engine + Bundled Model + Launch Prerequisites (v0.1 → v0.4)

**Release tag**: `v0.1.0` → `v0.4.x`
**PRD reference**: Sections 4, 6, 11, 12 (Weeks 1–6 of PRD Phase 1) + B6 from review + brought-forward items (B1, `vektor init`, C8.1, C8.2, lightweight benchmark, README/LICENSE)
**Depends on**: Stage 1 complete
**Effort**: Large (timeline lifted; estimate 14–18 weeks part-time for a Rust learner — slightly larger than v2.5 estimate to absorb the brought-forward items)
**Status**: In progress — `v0.1.0` skeleton implemented locally; publish pending

### Goal
A working MCP server that can index a real codebase, perform hybrid BM25 + semantic search, and assemble token-budgeted context — **with the security, distribution, and launch prerequisites a stranger needs to actually use the binary on day one.** No more "alpha that only the author can run."

### Deliverables — Core Engine (PRD Phase 1)

Follows PRD Section 12 weekly breakdown. Cargo releases tag at natural completion points:

| Sub-stage | Release | What ships (core engine) | What ships (launch prereqs — see brought-forward table) | PRD ref |
|---|---|---|---|---|
| Skeleton | `v0.1.0` | `cargo build` succeeds, CLI parses `vektor index` and `vektor serve`, no-op handlers respond to MCP | README.md, LICENSE, basic GitHub Actions CI (build+test on macOS+Linux) | Week 1 — **Function 1.1 only** (binary entry point). PRD Functions 1.2 (`discover_files`), 1.3 (`hash_file`), 1.4 (`HashStore`) defer to `v0.2.0` (tasks 2.1 + 2.2). |
| AST chunker | `v0.2.0` | Tree-sitter chunking for 5 languages + sliding-window fallback. Output verified via `vektor index --dump-chunks` | — | Week 1 Functions 1.2–1.4 (`discover_files`, `hash_file`, `HashStore`) brought forward from `v0.1.0` per the phase-1 scope cut + Week 2 Functions 2.1–2.5 (chunker proper) |
| Embed + vector store | `v0.3.0` | OnnxEmbedder loads Jina v2, embeds chunks, stores in LanceDB. `vektor index` produces a queryable LanceDB table | `SecretDetector` skips secrets during indexing (B1.2) | Week 3, Function 3.1–3.9 |
| BM25 + hybrid search + MCP | `v0.4.0` | Tantivy BM25 + RRF fusion + MCP server bootstrap + 3 primary tool handlers (`index_codebase`, `search_code`, `get_context_for_prompt`) | `vektor init` MCP config writer, release artifacts (binaries + checksums) for macOS+Linux, lightweight benchmark gate (≥20 queries against tokio) → BENCHMARKS.md v0.1 | Week 4–6, Function 4.1–4.8 + CA.1–CA.6 |

### Deliverables — B6: Bundled Model Offline Path

Runs in parallel with the engine work (no dependency on engine internals):

| # | Item | What it produces |
|---|---|---|
| B6.1 | `vektor models download [--lite]` subcommand | Idempotent download of Jina v2 (default) or bge-small (`--lite`) to `~/.vektor/models/`. Resumable on partial failure. Progress bar via `indicatif`. |
| B6.2 | `--model-path` flag | Overrides `~/.vektor/models/` to a user-specified directory. Supports airgapped environments. |
| B6.3 | `VEKTOR_MODELS_DIR` env var | Same as `--model-path` but via env (Docker/CI use). |
| B6.4 | Release tarball strategy | GitHub Release assets include `vektor-<platform>-with-jina-v2.tar.gz` (binary + model, ~330MB) and `vektor-<platform>.tar.gz` (binary only, ~30MB). Crates.io install users get the binary after the final crate name is chosen, then run `vektor models download` on first launch. |
| B6.5 | Mirror fallback for HF | If HuggingFace download returns 429/timeout, fall back to a list of mirrors (CDN, ModelScope, etc.). Config in `~/.vektor/config.toml`. |

### Deliverables — Brought forward from external review (2026-05-27)

The reviewer correctly argued that secret handling, MCP server registration, prebuilt binaries, benchmark proof, and README/LICENSE are *launch prerequisites*, not v1.0 nice-to-haves. They join Stage 2, with source-install docs at `v0.1.0` and prebuilt binaries at `v0.4.0`:

| # | Item | Ships in | What it produces |
|---|---|---|---|
| **B1.2** | `SecretDetector` module (moved from Stage 4) | `v0.3.0` | Static gitleaks-style rules + Shannon-entropy check. Chunks containing matches are skipped before embedding, logged as warnings, and surfaced in `index_codebase` response metadata. Inexpensive (~50 regexes, ~5ms per file). The full PRD Section 4.11 doc + audit subcommand (B1.1/B1.3/B1.4) still ship at Stage 4 — this is the *behavior*, not the *governance*. |
| **B1.5** | `.env*`, `*.pem`, `*.key`, `credentials.json` file-level skip list | `v0.3.0` | Hardcoded skip list in the file discovery path. Doesn't even read these files into memory. |
| **L1** | `vektor init` MCP config writer | `v0.4.0` | Subcommand that detects installed agents (Claude Code via `~/.claude.json`, Cursor via `~/.cursor/mcp.json`, Codex via `~/.codex/config.toml`) and writes the appropriate MCP server entry. Reviewer's point: "Skills teach agents *when* to call Vektor; they don't register the server." Without `vektor init`, "zero-config" is a half-truth. |
| **L2** | README.md | `v0.1.0` | Project pitch, current state, source install path (`cargo install --git ... --tag v0.1.0`), quickstart (3 commands max), links to PRD/ROADMAP, license. Prebuilt binary install docs are added at `v0.4.0` with the release-artifacts pipeline. |
| **L3** | LICENSE (MIT) | `v0.1.0` | The license already declared in PRD Section 1 + `Cargo.toml`. Shipping the actual file matters for `cargo publish` and for OSS-tooling correctness. |
| **C8.1** | GitHub Actions CI matrix (moved from Stage 6) | `v0.1.0` (lite) → `v0.4.0` (full) | At `v0.1.0`: build + test on macOS-aarch64 and linux-x86_64. At `v0.4.0`: full matrix adds macOS-x86_64, linux-aarch64, windows-x86_64. Reviewer's point: "most target users need prebuilt binaries; defaulting to Cargo + 300MB model + Rust 1.88 toolchain is friction." |
| **C8.2** | Release artifacts pipeline (moved from Stage 6) | `v0.4.0` | GitHub Release on tag push: signed binaries + SHA-256 checksums for each platform in the matrix, plus the `--with-jina-v2` tarballs from B6.4. Homebrew tap, AUR, deb/rpm stay at Stage 6 (`C8.3`). |
| **BM1** | Lightweight benchmark gate at `v0.4.0` | `v0.4.0` | A 20-query labeled subset of the eventual C1 corpus, run against tokio's source code. Outputs to `BENCHMARKS.md`. Establishes the baseline that subsequent releases must not regress. Reviewer's point: "claims need proof; BENCHMARKS.md needs to be part of launch, not later." |

The full versions of items B1, C8, and C1 still live in their original stages (Stage 4 for B1.1/B1.3/B1.4 doc + audit; Stage 6 for C8.3+ packaging; Stage 3 for full C1 corpus). Stage 2 just pulls forward the *minimum viable subset* that the reviewer correctly argued must exist on day one.

### Exit criteria (must all be true to ship v0.4)

**Core engine:**
- [ ] `vektor index /some/repo` completes without errors on at least 3 real repos (FastAPI source, tokio, a small TypeScript project)
- [ ] `vektor serve` registers `index_codebase`, `search_code`, `get_context_for_prompt` with Claude Code via MCP and they all return correct JSON
- [ ] Documented install path works end-to-end on a fresh macOS install (`cargo install --git ... --tag ...` until the crates.io name is resolved; crates.io install after rename/publish)
- [ ] Test coverage > 70% on the chunker, embedder, and search modules (`cargo tarpaulin` or equivalent)
- [ ] No `unwrap()` calls outside `#[cfg(test)]` blocks (per PRD Section 15 Implementation Rules)
- [ ] All Section 14 "Phase 1 Must Have" criteria pass for the 3 tools shipped (the other 5 workflow tools are explicitly deferred to Stage 3)

**Launch prereqs (brought forward from review):**
- [ ] `vektor init` writes correct MCP config for at least Claude Code and Cursor on macOS+Linux
- [ ] Secret-aware indexing: planting `AWS_ACCESS_KEY_ID=AKIA...` and a fake `id_rsa` block in a test repo and indexing it — both must be skipped, with a warning logged
- [ ] CI matrix: `cargo build --release && cargo test` green on macOS-aarch64 + linux-x86_64 on every PR; `v0.4.0` tag produces downloadable signed binaries for both
- [ ] `BENCHMARKS.md` exists with `v0.4.0 baseline` numbers from at least 20 labeled queries against tokio
- [ ] `README.md` walks a fresh user from `curl install` (or `cargo install`) → `vektor init` → first MCP call in under 5 commands
- [ ] `LICENSE` file present, matches the MIT license declared in PRD Section 1

### Open decisions

- **D2.1** — Should crates.io install auto-trigger `vektor models download` on first MCP server start after the final crate name is chosen? Reduces friction; adds 5-minute first-run latency. *Recommendation: prompt-then-download with `--auto-download` to skip the prompt.*
- **D2.2** — Bundle model in the cargo crate itself (makes `cargo install` download 330MB) or strictly via GitHub releases (cargo users always need a second step)? *Recommendation: GitHub releases only; cargo crate stays lean.*
- **D2.3** — Which 3 reference repos for the exit criteria? *Recommendation: FastAPI (Python, ~700 files), tokio (Rust, ~400 files), a small TypeScript repo TBD.*
- **D2.4** — Which secret-detection rule set for B1.2? *Recommendation: a curated subset (~50 rules) from [gitleaks](https://github.com/gitleaks/gitleaks)' default config, vendored as a static `secret_rules.toml`. Updating the rule set ships with each Vektor release; users don't need to fetch rules separately.*
- **D2.5** — `vektor init` strategy when a user already has an MCP config for Vektor: refuse, prompt to overwrite, or merge? *Recommendation: refuse with a clear message + `--force` flag to overwrite.*

---

## Stage 3 — Phase 1.5 + C1: Workflow Tools + Benchmark Corpus (v0.5 → v0.9)

**Release tag**: `v0.5.0` → `v0.9.x`
**PRD reference**: Section 8 (Coding Workflow Tools), Sections CA.7–CA.17 in Section 12, + C1 from review
**Depends on**: Stage 2 complete (the 3 primary tools work end-to-end)
**Effort**: Large (workflow tools are 5 sub-products; corpus is 2-week labeling task)
**Status**: Not started

### Goal
All 8 MCP workflow tools return useful, well-typed responses against a labeled benchmark corpus that we can measure against. This is where Vektor becomes *measurable*, not just *running*.

### Deliverables — C1 Benchmark Corpus (build this *first* in Stage 3)

This unlocks every later quality claim. Do not start the workflow tools until C1 exists.

| # | Item | What it produces |
|---|---|---|
| C1.1 | `benchmarks/` directory in repo | `corpus/`, `queries/`, `runner/`, `results/` subdirs |
| C1.2 | 3 reference codebases checked in as git submodules | FastAPI, tokio, plus one TypeScript repo at ~2K files |
| C1.3 | 50 ground-truth queries per codebase (150 total) | `queries/{repo}/queries.toml` — each entry has `query`, `relevant_chunks`, `borderline_chunks`, `irrelevant_chunks` |
| C1.4 | `cargo bench --bench precision_at_5` harness | Runs queries through Vektor's 8 tools, computes Precision@5 / Recall@5 / MRR + workflow-specific metrics |
| C1.5 | `BENCHMARKS.md` template | Auto-generated from latest bench run. Tracks numbers per release tag. |

### Deliverables — Workflow Tools (5 new MCP tools)

Each one is its own sub-product. Cargo releases tag at natural completion:

| Sub-stage | Release | What ships | PRD ref |
|---|---|---|---|
| Benchmark corpus | `v0.5.0` | C1.1–C1.5 above; runs against `v0.4` 3 tools to establish baseline | C1 |
| `get_context_for_task` | `v0.6.0` | All 8 `task_type` variants work (implement / debug / fix_test / review_diff / refactor / explain / write_tests / security_review). Thin wrapper over context assembly. | CA.13 |
| `get_context_for_diff` + `find_relevant_tests` | `v0.7.0` | Unified-diff parser, related-test detection, configs/schemas adjacency. Risk summary uses mechanical heuristics only (no LLM). | CA.14, CA.16 |
| `get_context_for_error` | `v0.8.0` | Stack trace / compiler / test-failure parsers for Python, Rust, Go, TS. Owner-module inference. | CA.15 |
| `get_project_overview` | `v0.9.0` | Language, framework, entry-point, command, architecture detection. Manifest-driven (`pyproject.toml`, `Cargo.toml`, `package.json`, `Makefile`). | CA.17 |

### Exit criteria (must all be true to ship v0.9 → v1.0 stage)

- [ ] All 8 MCP workflow tools registered and respond with PRD-compliant JSON
- [ ] Each workflow tool has at least 5 dedicated queries in the C1 corpus and Precision@5 ≥ 0.6 on them
- [ ] `get_context_for_diff` risk summary is **clearly mechanical** (e.g., *"changed file has N callers, M tests, K adjacent configs"*) — no LLM inference claimed
- [ ] `BENCHMARKS.md` shows per-release-tag numbers from `v0.5` through `v0.9`
- [ ] Skills file (`.claude/skills/vektor/SKILL.md`) shipped and discoverable by Claude Code
- [ ] First-run experience (Section 7.3) works end-to-end on a fresh install

### Open decisions

- **D3.1** — Risk-summary for `get_context_for_diff`: PRD example says *"JWT expiry behavior changed; login/session tests are likely affected."* This implies LLM inference. Acceptable Phase-1.5 outputs are *"changed file is `src/auth/jwt.py`; 3 callers in `src/api/*.py`; 2 directly-imported tests; 1 adjacent config (`config/auth.yml`)."* **Decision needed**: stick with the mechanical version or add an optional LLM hook in v0.7? *Recommendation: mechanical-only in v0.7; LLM hook becomes a Stage 5 (v1.x) item gated on user demand.*
- **D3.2** — Who labels the 150 queries? Solo labeling is ~25 hours of careful work. *Recommendation: do it yourself across two weekends; treat it as a research budget item, not a stretch goal.*
- **D3.3** — Public release of the benchmark corpus? It's a reusable artifact for the OSS ecosystem. *Recommendation: yes — separate `vektor-benchmarks` repo, CC-BY-4.0. Builds OSS credibility.*

---

## Stage 4 — v1.0 Cutoff: Security + Recovery (v1.0.0)

**Release tag**: `v1.0.0`
**PRD reference**: B1, B2, B5 from review (new sections to add to PRD before cutting `v1.0`)
**Depends on**: Stage 3 complete
**Effort**: Medium-Large
**Status**: Not started

### Goal
Make Vektor responsible enough for stranger-installs-it daily use. This is the gate between "alpha" and "production-grade."

### Deliverables — B1: Secret-Aware Indexing (governance + audit; behavior already shipped at v0.3)

> **Note (2026-05-27):** The *behavior* (B1.2 + B1.5) was pulled forward into Stage 2 (`v0.3.0`) per external review — Vektor refuses to embed obvious secrets from day one. What stays here is the *governance and audit surface*: the formal PRD doc, the report flag, and the retroactive audit subcommand.

| # | Item | What it produces |
|---|---|---|
| B1.1 | New PRD Section 4.11 "Secret-aware indexing" | Formal specification of what Vektor refuses to embed and the detection rules. The implementation already exists since `v0.3.0`; B1.1 documents the contract. |
| ~~B1.2~~ | ~~`SecretDetector` module~~ | **Moved to Stage 2 / `v0.3.0`** — see Stage 2 brought-forward table. |
| B1.3 | `vektor index --report-secrets` flag | Dry-run mode that lists what *would* be skipped. Helps users audit their repo before committing to an index. |
| B1.4 | Existing-index secret audit | `vektor audit-secrets` scans an existing index for chunks that retroactively match secret patterns (e.g., rule set updated). Removes them, logs what was removed. |
| ~~B1.5~~ | ~~File-level skip list~~ | **Moved to Stage 2 / `v0.3.0`** — `.env*`, `*.pem`, `*.key`, `credentials.json` skipped during file discovery. |

### Deliverables — B2: Threat Model + Privacy Guarantees

| # | Item | What it produces |
|---|---|---|
| B2.1 | New PRD Section 4.12 "Threat model" | Enumerates threats: stolen laptop, multi-user machine, malicious MCP client, sidecar processes reading `~/.vektor/`. For each: assumption, exposure, mitigation. |
| B2.2 | Encryption-at-rest for `~/.vektor/` | Optional, opt-in via `[security] encrypt_index = true`. Uses OS keyring for the key. |
| B2.3 | MCP authentication for SSE transport | Token-based auth for `vektor serve --sse`. stdio mode remains unauthenticated (per protocol). |
| B2.4 | `vektor wipe` subcommand | Destructive: zeroes out all `~/.vektor/{project_hash}/` data. For "laptop being decommissioned" scenarios. |
| B2.5 | `PRIVACY.md` | Publicly-stated privacy guarantees. What Vektor sends out (nothing, by default). What it stores (chunks + embeddings). User-visible audit path. |

### Deliverables — B5: Cross-Store Consistency Test Plan

| # | Item | What it produces |
|---|---|---|
| B5.1 | `tests/chaos/` directory | Tests that inject crashes at each of the 6 steps in Section 4.5's delete-then-insert flow |
| B5.2 | `cargo test --test chaos_recovery` | Verifies HashStore-as-source-of-truth recovers cleanly from each injection point |
| B5.3 | Concurrent-write test | Two threads calling `reindex_file` on the same path concurrently — should not corrupt LanceDB or Tantivy |
| B5.4 | OOM-during-embed test | Simulate OOM mid-batch — verify clean rollback, no partial-batch persisted |
| B5.5 | Disk-full test | Simulate ENOSPC during LanceDB or Tantivy commit — verify rollback semantics |

### Exit criteria (must all be true to ship v1.0)

- [ ] PRD sections 4.11, 4.12 + `PRIVACY.md` published; no claimed privacy property lacks a code-level reference
- [ ] Secret-detection scans a deliberately-poisoned test repo (with planted AWS keys, GitHub tokens, PEM blocks) and skips 100% of them
- [ ] Chaos test suite: 5+ crash-injection points × 3+ recovery scenarios = ≥15 tests, all green
- [ ] Concurrent / OOM / disk-full tests green on macOS + Linux
- [ ] No regression on Stage 3 benchmark numbers (`BENCHMARKS.md` v1.0 ≥ v0.9 on Precision@5)

### Open decisions

- **D4.1** — Encryption-at-rest default: on or off? On increases first-run latency (keyring prompt) but matches the "privacy is the differentiator" pitch. *Recommendation: off by default, prominently documented; on by default reconsidered for v1.x once UX is smooth.*
- **D4.2** — Threat-model scope: include nation-state adversaries or stick to "casual local-machine attacker"? *Recommendation: scope to "casual local attacker + colocated process." Nation-state is out of scope for a local dev tool.*

---

## Stage 5 — v1.x Trust Layer: Calibration + Feedback + Composition

**Release tag**: `v1.1.0` → `v1.x`
**PRD reference**: C2, C3, C4 from review (new sub-sections to add to PRD Section 5)
**Depends on**: Stage 4 (v1.0 shipped)
**Effort**: Medium
**Status**: Not started

### Goal
Turn the working tool surface from Stage 3 into a *trusted* surface. Agents should rely on Vektor's confidence signals, learn from passive feedback, and benefit from tool-to-tool composition.

### Deliverables — C2: Calibrate `result_confidence` and `missing_context_warnings`

| # | Item | What it produces |
|---|---|---|
| C2.1 | Calibration study against C1 corpus | For each Confidence label (High/Medium/Low), measure agreement with corpus ground truth. |
| C2.2 | Updated thresholds | Replace the hardcoded "top score >0.8" rule with empirical cutoffs derived from C2.1. |
| C2.3 | `vektor calibrate` subcommand | Re-runs calibration against the user's own labeled queries (Stage 6 territory but seeded here). |
| C2.4 | Updated PRD Section 5.3 | `Confidence` enum doc updated with the calibrated cutoffs + the date/dataset they came from. |

### Deliverables — C3: Activate Feedback Loop Properly

| # | Item | What it produces |
|---|---|---|
| C3.1 | New PRD Section 5.13 "Passive feedback signals" | Specifies which signals to consume: query re-issue within 30s, file-read-after-result, file-edit-after-result. |
| C3.2 | `FeedbackInferrer` module | Watches MCP request stream for the patterns; records inferred useful/irrelevant signals. |
| C3.3 | Symmetric explicit + passive ingestion | `report_context_quality` (explicit) and `FeedbackInferrer` (passive) write to the same `feedback.db`. |
| C3.4 | Per-project feedback dashboard | `vektor feedback-stats` CLI shows top-N most-useful and top-N most-skipped chunks. |

### Deliverables — C4: Tool Composition

| # | Item | What it produces |
|---|---|---|
| C4.1 | New PRD Section 8.4 "Tool composition graph" | Documents which workflow tools call which engine primitives and each other. |
| C4.2 | Refactor: extract shared `WorkflowEngine` | `get_context_for_error` internally calls `find_relevant_tests`. `get_context_for_diff` calls `WorkflowEngine::risk_summary`. No duplicated heuristics across tool handlers. |
| C4.3 | Integration tests | Tests that verify composition behavior: error referencing `tests/test_X.py` produces same tests as `find_relevant_tests(target_files=["tests/test_X.py"])`. |

### Exit criteria (must all be true to ship v1.x → v2.0)

- [ ] `result_confidence == High` agrees with corpus ground truth ≥85% of the time
- [ ] Feedback loop is producing measurable ranking adjustments (positive delta on re-run benchmark queries vs `v1.0` baseline)
- [ ] No workflow tool handler is more than 100 lines (composition has eliminated duplication)
- [ ] `BENCHMARKS.md` shows monotonic improvement v1.0 → v1.x on at least 2 of the 5 workflow-specific metrics

### Open decisions

- **D5.1** — Passive feedback inference requires Vektor to know about agent-side actions (file reads, edits). MCP doesn't expose this directly. Two options: (a) add an explicit `vektor://activity` resource that agents push to (cooperative), or (b) infer from access patterns on `~/.vektor/` and watcher events. *Recommendation: (a) cooperative — document the protocol; default to (b) if agents don't adopt it.*
- **D5.2** — Should we ship a "calibration regression alert" that warns on each `v1.x` release if Confidence accuracy drops? Adds CI complexity. *Recommendation: yes, as a Stage 6 (CI/CD) item.*

---

## Stage 6 — v2.x Platform: Streaming, Cost, Schema, CI/CD, Plugin, Telemetry

**Release tag**: `v2.0.0` → `v2.x`
**PRD reference**: C5–C10 from review (new sections / new PRD top-level area)
**Depends on**: Stage 5 (v1.x shipped, trust layer proven)
**Effort**: Large but each item is independently scopable
**Status**: Not started

### Goal
Turn Vektor from a tool into a platform. Each item below is a deliverable independent of the others — they can ship in any order across the `v2.x` series.

### Deliverables — C5: Streaming Responses

| # | Item | Effort |
|---|---|---|
| C5.1 | MCP server emits partial `ContextPackage` updates as chunks are assembled | M |
| C5.2 | Latency improvement validated against benchmark: time-to-first-chunk should be ≥40% lower than time-to-full-response for `get_context_for_prompt` | S |
| C5.3 | Updated PRD Section 9 with streaming response schema | S |

### Deliverables — C6: Cost Transparency for Cloud Backends

| # | Item | Effort |
|---|---|---|
| C6.1 | Token-counting in `index_codebase` dry-run mode (`--dry-run`) — estimates cost before any API call | S |
| C6.2 | `estimated_cost_usd` field in `index_codebase` response when using cloud backends | S |
| C6.3 | Per-backend cost table in PRD Section 6 | S |
| C6.4 | `vektor cost-report` subcommand — sums up API spend since last report | M |

### Deliverables — C7: Schema-Versioned Tool Surface

| # | Item | Effort |
|---|---|---|
| C7.1 | Add `vektor_schema_version` field to every MCP tool response | S |
| C7.2 | Document back-compat policy: additive changes are minor, renames/removals are major | S |
| C7.3 | Schema-version mismatch warning emitted to agents on stale clients | S |

### Deliverables — C8: CI/CD Matrix (packaging surfaces; core CI already shipped at v0.4)

> **Note (2026-05-27):** Basic CI matrix (C8.1) and release artifacts (C8.2) were pulled forward into Stage 2 per external review. At `v0.1.0`, CI proves source builds and the release is notes-only. Signed downloadable binaries start at `v0.4.0`. What stays here is the *packaging-channel* expansion (Homebrew/AUR/deb/rpm) and the regression-gate hooks that depend on benchmark/calibration infrastructure built in earlier stages.

| # | Item | Effort |
|---|---|---|
| ~~C8.1~~ | ~~GitHub Actions build matrix~~ | **Moved to Stage 2 / `v0.1.0` (lite) → `v0.4.0` (full)** |
| ~~C8.2~~ | ~~Release artifacts: signed binaries + checksums~~ | **Moved to Stage 2 / `v0.4.0`**. SBOM stays here (advanced supply-chain signal — see C8.6 below). |
| C8.3 | Homebrew tap (`brew install vektor/tap/vektor`), AUR package, deb/rpm | L |
| C8.4 | Per-PR benchmark regression check against C1 corpus | M |
| C8.5 | Per-release calibration check (C2 calibration must not degrade) | S |
| C8.6 | SBOM generation (CycloneDX) + Sigstore signing | M |

### Deliverables — C9: Plugin Model

| # | Item | Effort |
|---|---|---|
| C9.1 | `ChunkSource` trait extracted from Chunker | M |
| C9.2 | External-crate registration via `inventory` or `linkme` | M |
| C9.3 | Example plugin: `vektor-source-jupyter` (index Jupyter notebook cells) | M |
| C9.4 | PRD Section 16 "Plugin model" documenting the trait surface and lifecycle | M |

### Deliverables — C10: Opt-In Telemetry

| # | Item | Effort |
|---|---|---|
| C10.1 | `~/.vektor/telemetry.toml` opt-in config | S |
| C10.2 | Anonymized event schema (tool-call counts, latency histograms, error types) — never code content | M |
| C10.3 | Public telemetry dashboard (aggregate stats) | M |
| C10.4 | PRD Section 4.13 "Telemetry" — what's collected, where it goes, how to disable | S |

### Deliverables — Contributor Infrastructure (gated on v1.0 cutoff per AskUserQuestion answer)

| # | Item | Effort |
|---|---|---|
| Con.1 | `CONTRIBUTING.md` | S |
| Con.2 | Issue templates (bug, feature, security) | S |
| Con.3 | PR template + code-review checklist | S |
| Con.4 | `ARCHITECTURE.md` deep dive for new contributors | M |
| Con.5 | `good-first-issue` labeled backlog | M |

### Exit criteria

Stage 6 is not a single release — it's a series. Per-item exit criteria are in the deliverable description. The Stage as a whole completes when there's no item left in the C5–C10 list that's still listed as "Not started."

### Open decisions

- **D6.1** — Plugin model: is it worth the complexity? *Recommendation: defer until we have a real user asking for it. Don't build a plugin system without a first plugin customer.*
- **D6.2** — Telemetry: opt-in vs opt-out? OSS norm is opt-in. *Recommendation: opt-in only, documented prominently.*
- **D6.3** — Contributor infra placement: with C8 (CI/CD) feels natural but could ship earlier. *Per AskUserQuestion answer: align with v1.0 cutoff. Move to end of Stage 4 if Stage 4 work goes faster than expected.*

---

## Cross-Cutting Concerns

These run alongside every stage, not as a stage of their own:

### Documentation discipline
- `README.md` must work on a fresh machine at every release tag (verified by Stage 6 CI)
- `BENCHMARKS.md` updated at every release tag
- PRD changes are commits, not retcons — preserve history of design evolution

### Test discipline
- PRD Section 15 rule: every function has a `#[test]` before moving to the next
- Stage 4 chaos test suite runs in CI from v0.5 onwards (`cargo test --test chaos_recovery`)
- Benchmark suite runs at every PR from Stage 3 onwards

### Commit discipline
- PRD Section 15 rule: one function per commit, `cargo check` must pass
- Conventional commit format: `feat(chunker): add extract_chunks_ast for Python`
- Per global CLAUDE.md: if branch has a JIRA ticket, include it in commit message (no Claude co-author tag)

### Versioning discipline
- `Cargo.toml` version bumps follow the staged plan above
- Pre-1.0: minor bumps may break; major bumps reserved for `v1.0` cutoff and `v2.0` platform shift
- Post-1.0: strict semver

---

## Open Questions Across All Stages

Items where I'd want a decision before the relevant stage begins:

| # | Question | Latest stage to decide by |
|---|---|---|
| Q1 | Reference repos for benchmark corpus (FastAPI confirmed; need TypeScript ~2K-file repo + does tokio remain the Rust choice or switch to something more representative of typical Rust apps like Axum?) | Stage 3 |
| Q2 | Whether to publish the benchmark corpus as `vektor-benchmarks` separate repo or keep in main | Stage 3 |
| Q3 | LLM hook for `get_context_for_diff` risk summaries — Stage 3 mechanical-only, or add optional LLM bridge in v0.7? | Stage 3 |
| Q4 | Encryption-at-rest default — off (current rec) or on? | Stage 4 |
| Q5 | Passive-feedback signal source — cooperative protocol or watcher inference? | Stage 5 |
| Q6 | Plugin model — defer indefinitely or budget for `v2.x`? | Stage 6 |

---

## How to Use This Document

- **At session start**: read the "Current State" table to know where things stand
- **Before starting work**: identify which Stage the work belongs to; check that the previous Stage's exit criteria are met
- **At every release tag**: update "Current State" + check off completed deliverables
- **When something surprising happens**: update the Open Decisions / Open Questions sections — these are the document's living parts
- **When a stage completes**: revise the next stage's deliverables based on what was learned

This document is meant to iterate. Don't treat it as a contract; treat it as a working plan that gets refined as reality intrudes.

---

*Vektor — Workflow-first context for AI coding agents. Built deliberately.*
