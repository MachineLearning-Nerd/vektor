# Task 3.2 — OnnxEmbedder construction and warm-up

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.2
**PRD reference**: Section 12 Function 3.2 (`OnnxEmbedder::new`), Section 6.1 local ONNX backend
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: L
**Depends on**: 3.1
**Blocks**: 3.3, 3.5, 3.11

## Objective

Add an ONNX embedder constructor that resolves model artifacts under the Vektor data directory, loads `tokenizer.json` and `onnx/model.onnx`, initializes an `ort` session, and performs warm-up before first real use.

## Inputs (must exist before starting)

- `Embedder` trait from task 3.1
- `Config.embedding.onnx_model` and `Config.index.data_dir`
- Existing `dirs` dependency and data-dir expansion behavior from `HashStore`
- `ort = "=2.0.0-rc.12"` and `tokenizers = "0.23"` in `Cargo.toml`

## Outputs (must exist after completion)

- `src/embedder/onnx.rs` with `OnnxEmbedder::new(config: &Config) -> Result<Self>`
- Deterministic model artifact path resolution under `config.index.data_dir/models/<safe-model-name>/`
- Tokenizer loading from `tokenizer.json`
- ONNX session loading from `onnx/model.onnx`
- Warm-up path that embeds dummy strings at batch sizes 1 and 32
- Unit tests for path resolution and missing-artifact errors that do not require network access

## Approach

- Reuse the same home-directory expansion semantics as `HashStore`; if duplication appears, extract a small shared helper rather than inventing a second expansion rule.
- Convert model names such as `jinaai/jina-embeddings-v2-base-code` into stable filesystem directories by replacing `/` with `--`.
- Treat task 3.11 as the owner of network downloads. This task should fail clearly when expected local artifacts are missing.
- Initialize the ONNX Runtime session with CPU support first; provider auto-selection can be added only if it compiles cleanly against the pinned `ort` version.
- Log selected provider/session metadata at debug or info level without printing to stdout.

## Acceptance criteria

- [ ] `OnnxEmbedder::new` loads local `tokenizer.json` and `onnx/model.onnx` from the resolved model directory
- [ ] Missing model artifacts return a clear `VektorError` message that tells the user to run `vektor models download`
- [ ] `OnnxEmbedder::name()` returns the configured model name
- [ ] `OnnxEmbedder::dim()` returns the model dimension for Jina v2 Base Code (`768`) and lite BGE (`384`)
- [ ] Warm-up runs during construction and failures are surfaced instead of deferred to first query
- [ ] Tests isolate `HOME`/`USERPROFILE` and do not touch the developer's real `~/.vektor`

## Verification

```bash
cargo test onnx_embedder
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- The canonical default artifact set is HuggingFace model `jinaai/jina-embeddings-v2-base-code` with `tokenizer.json` and `onnx/model.onnx`.
- `vektor models download --lite` uses BAAI BGE small artifacts in task 3.11; keep the constructor model-agnostic enough for both dimensions.
