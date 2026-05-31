# Task 3.11 — `vektor models download`

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.11
**PRD reference**: Roadmap B6.1, Section 6.1 model artifact strategy
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: M
**Depends on**: 3.2
**Blocks**: 3.12

## Objective

Replace the `vektor models download` stub with an idempotent, resumable model downloader that installs the default Jina v2 Base Code ONNX artifacts, or lite BGE artifacts with `--lite`, into the local Vektor model cache.

## Inputs (must exist before starting)

- CLI `models download [--lite]` surface from Phase 1
- Model path resolution from task 3.2
- `reqwest` and `indicatif` dependencies already present in `Cargo.toml`
- HuggingFace artifact layout for the supported model repositories

## Outputs (must exist after completion)

- Working `vektor models download` command
- Download target for default model: `config.index.data_dir/models/jinaai--jina-embeddings-v2-base-code/`
- Download target for lite model: `config.index.data_dir/models/BAAI--bge-small-en-v1.5/`
- `tokenizer.json` and `onnx/model.onnx` present after a successful download
- Idempotent behavior when files already exist and match expected size/checksum metadata
- Tests for path selection, idempotency, partial-file cleanup/resume, and error reporting without downloading large artifacts in default CI

## Approach

- Keep artifact metadata in code as a small static table containing model name, dimension, repository, artifact paths, and local filenames.
- Stream downloads to temporary `.part` files and atomically rename after success.
- Resume only when the server supports range requests and the existing partial file length is valid; otherwise delete the partial file and restart.
- Show progress on stderr so stdout remains script-friendly.
- After download, call the task 3.2 constructor in a smoke path to prove artifacts are loadable.

## Acceptance criteria

- [ ] `vektor models download` installs default model artifacts under the default model cache directory
- [ ] `vektor models download --lite` installs lite model artifacts under the lite model cache directory
- [ ] Existing complete artifacts are not re-downloaded
- [ ] Interrupted downloads do not leave corrupt final files
- [ ] Download failures include the URL/path that failed without exposing secrets
- [ ] Default tests use a local fixture server or mocked transport, not HuggingFace network access
- [ ] A documented ignored smoke test can validate the real HuggingFace artifacts manually

## Verification

```bash
cargo test models_download
cargo build
FAKE_HOME=$(mktemp -d)
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" ./target/debug/vektor models download --lite
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- The default model repository exposes `tokenizer.json` and `onnx/model.onnx`. Verify artifact paths against HuggingFace before implementing because model repositories can be reorganized.
- Keep `--lite` explicit in this task. Automatic low-RAM selection can be added only if it does not destabilize deterministic tests.
