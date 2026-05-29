# v0.1.0 - Skeleton release

Prepared: 2026-05-28

This is the first Vektor release milestone. It is a source-install, notes-only
release for validating the project skeleton, CLI shape, MCP server wiring, and
CI path before real indexing/search features are implemented.

No binary assets are attached to this release.

## What is included

- Rust crate metadata for `vektor` version `0.1.0`
- CLI skeleton with these commands:
  - `vektor index <path>`
  - `vektor serve`
  - `vektor models download`
- Config loading with default values, optional config file, and environment
  variable overrides
- Stderr-only tracing setup, including JSON logs for `vektor serve`
- rmcp stdio server bootstrap for `vektor serve --transport stdio`
- Three MCP tool declarations with no-op responses:
  - `index_codebase`
  - `search_code`
  - `get_context_for_prompt`
- MIT license, README, roadmap, and Phase 1 planning docs
- GitHub Actions CI coverage for the skeleton

## What is not included yet

This release does not index code, generate embeddings, write vector storage,
run BM25 search, assemble context, or provide production MCP tool behavior.
The MCP tools intentionally return "not implemented yet" responses.

This release also does not include:

- Prebuilt binaries
- crates.io publishing
- `vektor init`
- model download UX
- signed release artifacts

Those are planned for later roadmap milestones.

## Install from source

The supported install path for `v0.1.0` is Cargo install from the Git tag:

```bash
cargo install --git https://github.com/MachineLearning-Nerd/vektor --tag v0.1.0 --locked
vektor --help
```

## Prerequisites

Install Rust 1.91+ and Protocol Buffers before running `cargo install`.
The repository pins Rust 1.91.0 in `rust-toolchain.toml`.

Protocol Buffers is required because the LanceDB/Arrow dependency stack invokes
`protoc` during compilation.

Common install commands:

```bash
# macOS
brew install protobuf

# Debian/Ubuntu
sudo apt-get install protobuf-compiler

# Fedora
sudo dnf install protobuf-compiler
```

## Release visibility requirement

The repository must be public before publishing this release if the install
command is expected to work from an unauthenticated machine:

```bash
cargo install --git https://github.com/MachineLearning-Nerd/vektor --tag v0.1.0 --locked
```

## Publisher checklist

Before creating the tag and GitHub Release:

- Confirm the working tree is clean after the release commit.
- Push the release commit to `origin/main`.
- Wait for the `ci.yml` run on that exact commit to conclude `success`.
- Confirm the repository is public.
- Create and push the annotated `v0.1.0` tag.
- Create the GitHub Release using this file as the notes body.
- Attach zero release assets.

## Phase 2 unblocks

After `v0.1.0` is published, Phase 2 can start on file discovery, hashing,
language detection, tree-sitter parsing, AST chunking, fallback chunking, and
`vektor index --dump-chunks`.
