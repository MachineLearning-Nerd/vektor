# Task 3.4 — OpenAI-compatible embedder

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.4
**PRD reference**: Section 12 Function 3.4 (`OpenAiCompatEmbedder::embed`), Section 6.1 cloud API rate limiting
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: M
**Depends on**: 3.1
**Blocks**: 3.5

## Objective

Implement the cloud embedding backend for OpenAI-compatible `/v1/embeddings` APIs with configurable base URL, model name, API key, response parsing, rate limiting, and bounded retry on transient failures.

## Inputs (must exist before starting)

- `Embedder` trait from task 3.1
- `Config.embedding.openai_api_key`, `openai_base_url`, `openai_model`, and `max_requests_per_minute`
- `reqwest` and `serde_json` dependencies already present in `Cargo.toml`

## Outputs (must exist after completion)

- `src/embedder/openai.rs` with `OpenAiCompatEmbedder`
- Request and response structs for the OpenAI-compatible embeddings shape
- Retry/backoff behavior for HTTP 429 and 5xx responses
- Unit tests using a local test server or mock transport, with no real API calls in default tests

## Approach

- Build the endpoint by joining `openai_base_url` with `/embeddings` after normalizing a trailing `/v1`.
- Send `Authorization: Bearer <key>` only when the key is non-empty; empty keys should produce a clear config error for cloud backend use.
- Preserve input order when parsing response records, even if the API returns an explicit `index` field.
- Enforce `max_requests_per_minute` at request granularity so large indexing runs do not hammer low-tier API keys.
- Keep retry count bounded and make final errors include HTTP status and concise response body context.

## Acceptance criteria

- [ ] Backend sends `model` and `input` in the expected JSON request body
- [ ] Successful responses parse into `Vec<Vec<f32>>` in input order
- [ ] Missing API key returns a clear config error before making a request
- [ ] HTTP 429 and 5xx responses retry with exponential backoff and then fail cleanly if still unsuccessful
- [ ] Non-retryable 4xx responses fail without retry
- [ ] Tests cover success, missing key, retryable failure, and non-retryable failure
- [ ] No default test requires external network access or a real cloud API key

## Verification

```bash
cargo test openai_compat_embedder
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- This backend intentionally covers OpenAI-compatible providers broadly. Do not add a separate Ollama backend in Phase 3.
