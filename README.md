# Vektor

> Local-first **codebase context engine** as an MCP server.
> Token-budgeted, workflow-aware context for AI coding agents.
> 100% your machine. 100% open source.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Status](https://img.shields.io/badge/status-v0.4.0%20local--first%20closeout-yellow.svg)](VEKTOR_ROADMAP.md)
[![PRD](https://img.shields.io/badge/PRD-v2.5-blue.svg)](VEKTOR_PRD.md)

---

## What is Vektor?

Vektor is a high-performance, local-first **codebase context engine** exposed via the [Model Context Protocol (MCP)](https://modelcontextprotocol.io/). Built in Rust, it gives Claude Code, Cursor, Codex CLI, and any MCP-compatible AI coding agent task-specific, token-budgeted context for real coding work — without sending your code to the cloud.

A search engine returns ranked results. A coding context engine assembles **deduplicated, relationship-aware, token-budgeted context packages** optimized for an agent's active task. Vektor is the second kind.

When complete, Vektor will expose 8 primary MCP tools:

- `get_context_for_prompt` — broad token-budgeted context for a natural-language query
- `get_context_for_task` — context tuned for `implement_feature` / `debug_error` / `fix_test` / `review_diff` / `refactor` / `explain_code` / `write_tests` / `security_review`
- `get_context_for_diff` — changed files, related tests, affected configs/schemas, mechanical risk summary
- `get_context_for_error` — referenced code from a stack trace, compiler error, or test failure
- `find_relevant_tests` — likely unit / integration / E2E tests, fixtures, and suggested commands
- `get_project_overview` — languages, frameworks, entry points, commands, architecture areas
- `search_code` — raw hybrid BM25 + semantic search for agents that want direct access
- `index_codebase` — build or refresh the local index

The first three tools (`index_codebase`, `search_code`, `get_context_for_prompt`) ship in `v0.4.x`; the workflow tools follow across `v0.5.0` → `v0.9.x`. See [VEKTOR_ROADMAP.md](VEKTOR_ROADMAP.md) for staging.

---

## Status

**Pre-alpha — `v0.4.0` local-first launch polish implemented on the Phase 6 branch; public release pending authorization.**

The current artifacts are the design documents plus a runnable Rust implementation through the Stage 2 local-first scope:

| Document | Purpose | Lines |
|---|---|---|
| [`VEKTOR_PRD.md`](VEKTOR_PRD.md) | Product Requirements Document (v2.5). What and why. | ~2,700 |
| [`VEKTOR_ROADMAP.md`](VEKTOR_ROADMAP.md) | Staged implementation plan. When and how we know we're done. | ~500 |
| [`LICENSE`](LICENSE) | MIT license. | 21 |

The current codebase implements CLI parsing, config loading, stderr-only tracing, an rmcp stdio server, file discovery, file/chunk hashing, AST/sliding-window chunking, ONNX/OpenAI-compatible embedding backends, LanceDB vector storage, Tantivy BM25, hybrid search, real MCP handlers for the 3 primary tools, token-budgeted context assembly, `vektor init`, and a report-only retrieval benchmark baseline.

The repository and release artifacts remain private/deferred until the explicit `v0.4.0` release and `6.6 go public` authorization gates.

---

## Why local-first?

The two closest closed-source competitors (Augment Code, Sourcegraph Cody) charge $20–200/seat/month and require sending your code to Google Cloud or a Sourcegraph server. The closest open-source competitors (CocoIndex, Claude Context) either skip context assembly entirely (search-only) or require a cloud-hosted vector DB like Milvus.

Vektor's bet is that an individual developer running Claude Code or Cursor on their own laptop should not have to choose between (a) shipping code to a SaaS for indexing or (b) accepting search-only retrieval with no context assembly. We provide both, locally:

- **Embedded vector DB** ([LanceDB](https://github.com/lancedb/lancedb)) running in the Vektor process — no Docker, no sidecar daemon
- **Local embeddings** ([ONNX Runtime](https://onnxruntime.ai/) + [Jina v2 Base Code](https://huggingface.co/jinaai/jina-embeddings-v2-base-code)) — code never leaves the machine
- **Hybrid search** ([Tantivy](https://github.com/quickwit-oss/tantivy) BM25 + dense vectors via RRF fusion)
- **Context assembly layer** — what makes us different from search-only competitors

See [VEKTOR_PRD.md §2.3](VEKTOR_PRD.md) for the full competitive landscape.

---

## What `v0.1.0 → v0.4.x` will ship

Per [VEKTOR_ROADMAP.md](VEKTOR_ROADMAP.md), Stage 2 incrementally lands:

| Release | What works |
|---|---|
| `v0.1.0` | CLI skeleton, MCP no-op handlers, GitHub Actions CI (macOS+Linux), MIT license, README. Not useful yet — proves the build pipeline. |
| `v0.2.0` | Tree-sitter AST chunking for Python, TypeScript, JavaScript, Rust, Go. `vektor index --dump-chunks` shows what gets indexed. |
| `v0.3.0` | ONNX embedding via Jina v2, LanceDB storage, secret-aware indexing (skips `.env`, AWS keys, PEM blocks etc.). |
| `v0.4.0` | Tantivy BM25 + RRF hybrid search. MCP server with 3 tools. `vektor init` writes MCP config for Claude Code / Cursor. Signed binaries on GitHub Releases. Baseline `BENCHMARKS.md`. |

Workflow tools (`get_context_for_task` etc.) ship in Stage 3 (`v0.5.0` → `v0.9.x`).

---

## Prerequisites

Source installs build the LanceDB/Arrow stack. `lance-encoding` invokes `protoc` during compilation, so install Protocol Buffers before `cargo install`:

- macOS: `brew install protobuf`
- Debian/Ubuntu: `apt-get install protobuf-compiler`
- Fedora: `dnf install protobuf-compiler`
- Windows: `winget install protobuf` or `scoop install protobuf`

Also install Rust 1.91+; this repo pins 1.91.0 in `rust-toolchain.toml`.

## Install

Public install is not active yet. For local development from this checkout:

```bash
cargo build --release
./target/release/vektor --help
./target/release/vektor init --dry-run
```

The intended post-authorization `v0.4.0` flow is:

```bash
# Prebuilt binary after the GitHub Release is published
curl -fsSL https://install.vektor.dev | sh

# Cargo/crates.io install after the final crate name is approved
cargo install <final-crate-name>

# Then register with your agent
vektor init       # auto-detects Claude Code, Cursor, Codex CLI
vektor models download    # one-time ~300MB download of Jina v2

# Index your project
vektor index /path/to/repo
```

The public binary installer, crates.io package, release tag, and repository visibility flip are all deferred until explicit release/public authorization.

---

## Known issues / open decisions

- **Crate name `vektor` is already published on crates.io** (a SIMD utility crate). A rename is required before any crates.io publish. The `v0.1.0` milestone uses a Git tag install path and is not blocked by this decision. Candidates under discussion: `vektorctx`, `vektor-mcp`, or a clean rename. See [VEKTOR_ROADMAP.md](VEKTOR_ROADMAP.md) Known Issue note.
- **Single maintainer** (solo project). Issues and PRs welcome, but expect slower turnaround than a multi-maintainer project.
- **No timeline commitment.** Quality before speed. The roadmap describes staging, not delivery dates.

---

## Contributing

The project is still pre-alpha, so the most useful contribution today is *reviewing the docs and skeleton behavior*:

1. Read [VEKTOR_PRD.md](VEKTOR_PRD.md) and [VEKTOR_ROADMAP.md](VEKTOR_ROADMAP.md)
2. Open an issue if you spot inconsistencies, missing concerns, or scope problems
3. Bring evidence (`file:line` references, citations, benchmarks from comparable systems) — the project takes pointed criticism seriously

Once `v0.1.0` ships, a `CONTRIBUTING.md` will outline code-contribution norms (per [VEKTOR_ROADMAP.md](VEKTOR_ROADMAP.md) Stage 6).

---

## License

[MIT](LICENSE). See the license file for full text.

---

*Vektor — Workflow-first context for AI coding agents. Built deliberately.*
