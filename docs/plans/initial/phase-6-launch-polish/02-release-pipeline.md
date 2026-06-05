# Task 6.2 — Release pipeline, signing, checksums, and SBOM

## Objective

Create a tag-triggered release pipeline that builds the `v0.4.0` binary matrix and publishes signed artifacts with checksums and provenance.

## Scope

- Extend CI/release coverage to macOS x86_64/aarch64, Linux x86_64/aarch64, and Windows x86_64.
- Add `release.yml` triggered by version tag push.
- Generate SHA-256 checksum files.
- Sign release artifacts with a Sigstore-style workflow.
- Include a basic SBOM/provenance artifact if feasible for v0.4.0.

## Acceptance Criteria

- [ ] Tag push starts the release workflow.
- [ ] All 5 platform artifacts are built.
- [ ] Checksums match artifact bytes.
- [ ] Signatures/provenance are attached to the release.
- [ ] Failed platform jobs fail the release.

## Verification

```bash
cargo build --release
cargo test --workspace
gh workflow run release.yml --ref <test-tag-or-branch>
```
