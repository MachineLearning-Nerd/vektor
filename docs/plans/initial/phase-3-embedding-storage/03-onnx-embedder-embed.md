# Task 3.3 — OnnxEmbedder embedding

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.3
**PRD reference**: Section 12 Function 3.3 (`OnnxEmbedder::embed`), Section 6.1 direct `ort` + `tokenizers` implementation
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: L
**Depends on**: 3.2
**Blocks**: 3.5

## Objective

Implement ONNX embedding inference: tokenize prefixed texts, run the session, mean-pool token embeddings with the attention mask, L2-normalize vectors, and process inputs in bounded batches.

## Inputs (must exist before starting)

- `OnnxEmbedder` constructor from task 3.2
- `Embedder` prefix helpers from task 3.1
- Pinned `ort` and `tokenizers` crates
- PRD requirement that Jina v2 document/query prefixes are applied before tokenization

## Outputs (must exist after completion)

- `OnnxEmbedder` implements `Embedder`
- Batched embedding path with a default batch size of 32
- Mean-pooling and L2-normalization helpers with unit tests
- Integration-style test using a small fixture or ignored real-model test path

## Approach

- Let `embed_documents` and `embed_query` from the trait apply prefixes before this backend tokenizes.
- Construct model inputs from tokenizer output: `input_ids`, `attention_mask`, and `token_type_ids` only when the loaded model expects them.
- Keep pooling math in pure helper functions so it can be tested without loading ONNX Runtime.
- Verify each returned vector has length `dim()` and norm `1.0 +/- 1e-5`.
- Avoid retaining all model outputs longer than needed; large indexes will call this repeatedly.

## Acceptance criteria

- [ ] Embedding an empty input list returns an empty vector list without calling ONNX
- [ ] Embedding N texts returns N vectors
- [ ] Every vector length equals `OnnxEmbedder::dim()`
- [ ] Mean pooling uses the attention mask so padding tokens do not affect the vector
- [ ] Every non-zero vector is L2-normalized to `1.0 +/- 1e-5`
- [ ] Inputs are processed in batches of at most 32
- [ ] Tests cover pooling, normalization, empty input, and prefix-through-tokenization behavior

## Verification

```bash
cargo test onnx_embedder
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- If a real-model smoke test is too slow for default CI, mark only that smoke test `#[ignore]`; keep all math and shape tests in the default suite.
- Do not add OpenAI-compatible fallback here. Task 3.5 owns backend selection and fallback.
