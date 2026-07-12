# Task 6.2 — Release pipeline, signing, checksums, and SBOM

## Objective

Create local-verifiable release workflow scaffolding that builds the `v0.4.0` binary matrix and can publish signed artifacts with checksums and provenance when a release tag is later authorized.

## Scope

- Extend CI/release coverage to macOS x86_64/aarch64, Linux x86_64/aarch64, and Windows x86_64.
- Add `release.yml` triggered by version tag push, with manual dispatch support for dry-run validation.
- Generate SHA-256 checksum files.
- Sign release artifacts with a Sigstore-style workflow.
- Include a basic SBOM/provenance artifact if feasible for v0.4.0.

## Acceptance Criteria

- [x] Tag-push release workflow is scaffolded; live tag execution is deferred until release authorization.
- [x] All 5 platform artifacts are matrixed: macOS x86_64/aarch64, Linux x86_64/aarch64, and Windows x86_64.
- [x] SHA-256 checksum files are generated from packaged artifact bytes.
- [x] Sigstore signing, SBOM metadata, and provenance placeholders are attached to release artifacts.
- [x] Failed platform jobs fail the release because publish depends on the full build matrix.

## Verification

```bash
cargo build --release
cargo test --workspace
ruby -e 'require "yaml"; ARGV.each { |f| YAML.load_file(f); puts "ok #{f}" }' .github/workflows/ci.yml .github/workflows/release.yml
```

`gh workflow run release.yml --ref <test-tag-or-branch>` is intentionally deferred in the local-first Phase 6 plan because it requires pushing to GitHub before the final local gate.
