# Task 6.5 — `v0.4.0` release tag and announcement

## Objective

Cut the first launchable alpha release once init, release artifacts, signing, checksums, and benchmark baseline are complete.

## Scope

- Confirm crate-name/package decision before publishing.
- Tag `v0.4.0`.
- Publish the GitHub release with all artifacts.
- Smoke-test released binaries on the supported platform matrix.
- Update install documentation and publish the announcement.

## Acceptance Criteria

- [ ] `v0.4.0` tag exists and points at the intended commit.
- [ ] Release notes include install, model download, and `vektor init` instructions.
- [ ] All artifacts are attached, signed, and checksum-verified.
- [ ] Smoke tests pass for released binaries.
- [ ] Announcement links to the release and benchmark baseline.

## Verification

```bash
git tag --verify v0.4.0
gh release view v0.4.0
shasum -a 256 -c *.sha256
```
