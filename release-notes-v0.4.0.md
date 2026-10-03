# Vektor v0.4.0 Release Notes (draft)

> Draft only. Do not publish, tag, upload artifacts, publish to crates.io, or
> flip repository visibility until the separate release authorization gate is
> approved.

## Summary

`v0.4.0` is the first launch-polish milestone for Vektor's Stage 2 core engine.
It brings together the three primary MCP tools, token-budgeted context assembly,
agent registration via `vektor init`, release workflow scaffolding for signed
multi-platform binaries, and the first checked-in retrieval benchmark baseline.

## Highlights

- `vektor init` registers Vektor with Claude Code, Cursor, and Codex CLI MCP
  configs using idempotent writes, `--agent`, `--force`, and `--dry-run`.
- Context assembly now includes deduplication, related expansion, recency
  weighting, query caching, token-budget checks, confidence, gaps, and result
  clusters.
- Release workflow scaffolding builds the 5-platform matrix:
  macOS x86_64, macOS aarch64, Linux x86_64, Linux aarch64, and Windows x86_64.
- Release artifacts are configured for SHA-256 checksums, Sigstore signing,
  cargo-metadata SBOM output, and provenance placeholders.
- `BENCHMARKS.md` records the v0.4.0 launch baseline:
  `Precision@5=0.200`, `Recall@5=1.000`, `MRR=0.975` on 20 checked-in Tokio
  fixture queries.

## Install and first run

Public install commands remain deferred until the release gate is authorized.
The intended post-release flow is:

```bash
# Binary install after the GitHub Release is published.
curl -fsSL https://install.vektor.dev | sh

# Crates.io install only after the final crate name is approved.
cargo install <final-crate-name>

# Register Vektor with local MCP agents.
vektor init --dry-run
vektor init

# Download the local embedding model.
vektor models download

# Index a project before using MCP tools.
vektor index /path/to/repo
```

## Local verification before release authorization

Run from the repository root:

```bash
PROTOC=/tmp/vektor-protoc/protoc-35.0/bin/protoc cargo build
PROTOC=/tmp/vektor-protoc/protoc-35.0/bin/protoc cargo test --workspace
PROTOC=/tmp/vektor-protoc/protoc-35.0/bin/protoc cargo bench --bench retrieval_quality
cargo fmt --check
PROTOC=/tmp/vektor-protoc/protoc-35.0/bin/protoc cargo clippy --workspace --all-targets -- -D warnings
```

## Deferred release gates

- No `v0.4.0` tag is created in the local-first implementation batch.
- No GitHub Release is published in the local-first implementation batch.
- No crates.io publish happens until the crate-name decision is made.
- No repository visibility change happens until the explicit `6.6 go public`
  gate is approved.
