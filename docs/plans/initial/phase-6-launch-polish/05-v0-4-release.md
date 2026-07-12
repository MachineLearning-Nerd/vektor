# Task 6.5 — `v0.4.0` release readiness docs

## Objective

Prepare the first launchable alpha release docs once init, release workflow scaffolding, signing/checksum plumbing, and benchmark baseline are complete. This local-first task does not tag, publish, upload artifacts, publish to crates.io, or flip repository visibility.

## Scope

- Add draft `v0.4.0` release notes.
- Add a release authorization checklist with local gate, artifact checks, platform smoke tests, and the `6.6 go public` gate.
- Add the mandatory crate-name/package decision gate before any crates.io publish.
- Update install/readiness documentation without claiming the release is published.
- Explicitly defer tag creation, GitHub Release publication, crates.io publication, and public visibility changes.

## Acceptance Criteria

- [x] Draft release notes include install, model download, and `vektor init` instructions.
- [x] Release checklist documents artifact signing, checksum verification, and platform smoke tests.
- [x] Crate-name/package decision gate is documented before `cargo publish`.
- [x] Tag, GitHub Release, crates.io publish, and public visibility flip are explicitly deferred until separate authorization.
- [x] Announcement/release copy links to the benchmark baseline without claiming publication.

## Verification

```bash
test -f release-notes-v0.4.0.md
test -f release-checklist-v0.4.0.md
rg "Deferred release gates|Mandatory decision before `cargo publish`|Public visibility gate" release-notes-v0.4.0.md release-checklist-v0.4.0.md
```
