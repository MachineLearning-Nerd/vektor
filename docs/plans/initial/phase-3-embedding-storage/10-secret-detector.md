# Task 3.10 — SecretDetector and secret-aware indexing

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.10
**PRD reference**: Roadmap B1.2/B1.5, PRD privacy and secret-aware indexing requirements
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: M
**Depends on**: 2.1
**Blocks**: 3.7c

## Objective

Prevent obvious secrets from being embedded or sent to cloud APIs by adding a secret detector, file-level secret skip list, and indexing integration points that run before chunk embedding.

## Inputs (must exist before starting)

- Phase 2 file discovery and `vektor index` flow
- Phase 2 note that `.env` was intentionally visible until Phase 3
- Roadmap B1.2/B1.5 secret-aware indexing scope

## Outputs (must exist after completion)

- `src/secrets.rs` or `src/secret_detector.rs` with a `SecretDetector`
- File-level skip list for `.env`, `.env.*`, PEM/key files, npm/yarn/pnpm auth files, and common cloud credential files
- Regex and entropy checks for obvious secret values in chunks
- Tests proving file-level skips avoid reading secret files into memory
- Indexing hooks that skip secret files/chunks before embedding

## Approach

- Keep the rule set static and local; do not invoke external scanners during indexing.
- Apply file-level skip before reading file bytes.
- Apply content-level detection after safe files are read and chunked but before embedding.
- Log skipped path/chunk counts as warnings or structured fields without printing secret values.
- Keep the detector intentionally conservative enough to avoid embedding known credential shapes, even if that skips a little benign content.

## Acceptance criteria

- [ ] `.env` and `.env.*` files are skipped before file bytes are read
- [ ] PEM private keys, AWS access keys, GitHub tokens, Slack tokens, and generic high-entropy assignments are detected in content
- [ ] Secret values are never included in logs, stdout, test failure messages, or MCP responses
- [ ] `vektor index` reports skipped-secret counts
- [ ] Safe files continue to index normally
- [ ] Tests include the AWS sample key `AKIAIOSFODNN7EXAMPLE` and verify it is skipped before embedding
- [ ] Tests prove file-level skips are enforced before read by using a fixture that would fail if opened

## Verification

```bash
cargo test secret
cargo test index_cli
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Do not add a public audit subcommand in this task; the roadmap keeps broader privacy governance for a later stage.
