# Task 3.1 — Embedder trait

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.1
**PRD reference**: Section 12 Function 3.1 (`Embedder` trait), Section 6.2 query prefix strategy
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: S
**Depends on**: 2.9, 3.0
**Blocks**: 3.2, 3.4

## Objective

Define the shared async embedding backend contract that ONNX and OpenAI-compatible embedders implement. The trait owns document/query prefix handling so callers cannot accidentally embed code chunks and search queries with the wrong Jina v2 prefix.

## Inputs (must exist before starting)

- Phase 2 `Chunk` type and `chunk_file` dispatcher
- `async-trait` dependency already present in `Cargo.toml`
- PRD Section 12 Function 3.1 trait signature and Section 6.2 prefix rules

## Outputs (must exist after completion)

- `src/embedder/mod.rs` module exported from `src/main.rs`
- `Embedder` trait with `embed`, `dim`, `name`, `prefix_for_document`, and `prefix_for_query`
- Helper methods that apply document/query prefixes before delegating to backend implementation
- Unit tests proving prefixes are applied exactly once

## Approach

- Create the module before any backend implementation so later tasks compile against one stable interface.
- Keep the core `embed` method backend-facing and provide public helpers such as `embed_documents` and `embed_query`.
- Return `crate::error::Result<Vec<Vec<f32>>>` from all embedding paths.
- Use the embedding/network error variants introduced by task 3.0; do not add ad-hoc error categories here.
- Keep trait bounds `Send + Sync` so the embedder can be shared by async index/search orchestration.

## Acceptance criteria

- [ ] `Embedder` is async, object-safe, and usable behind `Box<dyn Embedder>`
- [ ] `embed_documents` prepends `prefix_for_document()` to every text before calling the backend
- [ ] `embed_query` prepends `prefix_for_query()` to the query text before calling the backend
- [ ] Empty prefixes leave text unchanged for models that do not need task prefixes
- [ ] Tests cover Jina-style prefixes and an empty-prefix backend
- [ ] The new module is exported in the binary crate without dead-code or clippy warnings

## Verification

```bash
cargo test embedder
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes

- Do not implement ONNX, HTTP, or factory behavior in this task.
- The prefix helper is the guardrail: callers should not manually concatenate `"search_document: "` or `"search_query: "` outside this module.
