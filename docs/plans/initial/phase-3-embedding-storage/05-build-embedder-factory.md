# Task 3.5 — Embedder factory

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.5
**PRD reference**: Section 12 Function 3.5 (`build_embedder`), Section 6.3 embedding configuration
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: S
**Depends on**: 3.2, 3.3, 3.4
**Blocks**: 3.7b

## Objective

Add a single factory that reads `config.embedding.backend` and returns the configured embedding backend as `Box<dyn Embedder>`, including explicit fallback-to-ONNX behavior when cloud backend initialization fails and fallback is enabled.

## Inputs (must exist before starting)

- `OnnxEmbedder` from tasks 3.2 and 3.3
- `OpenAiCompatEmbedder` from task 3.4
- `Config.embedding.backend` and `fallback_to_onnx`

## Outputs (must exist after completion)

- `build_embedder(config: &Config) -> Result<Box<dyn Embedder>>`
- Tests for `onnx`, `openai`, unsupported backend, and fallback decision paths
- Clear log message when fallback-to-ONNX is used

## Approach

- Accept backend values `onnx` and `openai`; reject `ollama` with a message that it is deferred.
- Try the configured primary backend first.
- If the primary backend is `openai`, initialization fails, and `fallback_to_onnx` is true, try ONNX once and log the fallback.
- If fallback is disabled, return the original backend error.
- Keep the factory free of indexing or vector-store behavior.

## Acceptance criteria

- [ ] `backend = "onnx"` returns an ONNX embedder
- [ ] `backend = "openai"` returns an OpenAI-compatible embedder when config is valid
- [ ] Unsupported backend values return a clear config error
- [ ] `backend = "ollama"` returns a clear deferred-backend error, not a silent fallback
- [ ] OpenAI initialization failure falls back to ONNX only when `fallback_to_onnx = true`
- [ ] Tests cover success, unsupported backend, fallback enabled, and fallback disabled

## Verification

```bash
cargo test build_embedder
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Do not instantiate embedders directly from CLI or storage code after this task; use the factory so fallback behavior stays centralized.
