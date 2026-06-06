# v0.4.0 Release Checklist (authorization gate)

This checklist is for the future release authorization step. It is not executed
by the local-first Phase 6 implementation batch.

## Mandatory decision before `cargo publish`

- [ ] Final crate/package name approved.
- [ ] `Cargo.toml` package name updated if it changes from `vektor`.
- [ ] README, roadmap, release notes, install docs, and PRD references updated
      to the approved crate name.
- [ ] crates.io dry run passes:

```bash
cargo publish --dry-run
```

Current state: unresolved. The name `vektor` is already taken on crates.io, so
crates.io publishing is blocked until this decision is made.

## Pre-release local gate

- [ ] `cargo build`
- [ ] `cargo test --workspace`
- [ ] `cargo bench --bench retrieval_quality`
- [ ] `cargo fmt --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `vektor init --dry-run` smoke test
- [ ] Temp-home `vektor init` write smoke for Claude Code, Cursor, and Codex

## Release workflow dry run

- [ ] Push a release-candidate branch after local gate approval.
- [ ] Run `.github/workflows/ci.yml` manually on the release-candidate branch.
- [ ] Run `.github/workflows/release.yml` manually with a non-published
      `release_tag` input.
- [ ] Confirm all 5 platform artifacts are produced.
- [ ] Confirm each artifact has a `.sha256`, `.sig`, `.pem`, SBOM metadata, and
      provenance JSON.

## Tag and GitHub Release

- [ ] Create the signed/annotated `v0.4.0` tag only after approval.
- [ ] Push the tag.
- [ ] Confirm tag-triggered `.github/workflows/release.yml` succeeds.
- [ ] Verify checksums:

```bash
shasum -a 256 -c *.sha256
```

- [ ] Verify GitHub Release contents and release notes.

## Platform smoke tests

Run on each release artifact platform:

- [ ] `vektor --help`
- [ ] `vektor init --dry-run`
- [ ] `vektor models download --lite`
- [ ] `vektor index <small-fixture-repo> --dump-chunks`
- [ ] MCP `tools/list` shows `index_codebase`, `search_code`, and
      `get_context_for_prompt`

## Public visibility gate (`6.6`)

- [ ] Curate `VEKTOR_PRD.md` and any competitive strategy notes before exposing
      the repository.
- [ ] Confirm unauthenticated install path on a fresh machine.
- [ ] Flip repository visibility only after explicit authorization.
