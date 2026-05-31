# Task 3.0 — Extend `VektorError` taxonomy for embedding/storage/network

**Phase**: 3 — Embedding + Storage
**Task ID**: 3.0
**PRD reference**: Section 12 (error handling conventions), Phase 1 Function 1.2 (`error` module)
**Roadmap stage**: Stage 2 / `v0.3.0`
**Effort estimate**: S  (S=≤1h)
**Depends on**: 1.2
**Blocks**: 3.1, 3.6

> **Provenance**: This task was added during the Phase 3 plan review (pre-implementation). The existing `VektorError` enum has no variant for the foreign error sources Phase 3 introduces (`ort`, `lancedb`, `reqwest`, `tokenizers`, `arrow`). Settling the taxonomy once, up front, prevents every downstream task from independently inventing `VektorError::State(e.to_string())` fallbacks.

## Objective

Extend the central `VektorError` enum so that embedding, vector-store, and network failures each have a dedicated, structured variant. This gives tasks 3.2–3.11 a consistent target for `?`/`map_err`, preserving the "fail fast with meaningful context" principle instead of stringly-typed error soup.

## Inputs (must exist before starting)

- `src/error.rs` with the existing `VektorError` enum (`Io`, `Config`, `NotImplemented`, `Parse`, `State`, `Mcp`) and `pub type Result<T>` alias — shipped in Phase 1 task 1.2
- `thiserror = "2"` already in `Cargo.toml`
- Awareness of the foreign error types Phase 3 will surface: `ort::Error`, `lancedb::Error`, `reqwest::Error`, `tokenizers` errors, and arrow errors

## Outputs (must exist after completion)

- New `VektorError` variants covering embedding, storage, and network failure categories (suggested: `Embedding(String)`, `Storage(String)`, `Network(String)` — final names at implementer's discretion, but they must be distinct from `State`)
- `#[from]` conversions where the foreign type implements `std::error::Error` ergonomically (e.g. `reqwest::Error`); a documented `map_err` helper pattern where `#[from]` is not clean (e.g. `ort`/`lancedb` whose error types may not be `'static + Send + Sync` as required)
- Updated `#[cfg(test)]` tests in `src/error.rs` covering `Display` output and `matches!` for each new variant

## Approach

- Add the variants with `#[error("...: {0}")]` Display messages consistent with the existing style (lowercase category prefix, e.g. `"embedding error: {0}"`).
- Prefer `#[from]` only for foreign error types that satisfy thiserror's bounds; for types that don't, keep the variant as `Storage(String)` / `Embedding(String)` and map at the call site with a short helper (mirroring how `config_error_to_vektor` wraps `config::ConfigError`).
- Do NOT introduce new dependencies. Do NOT touch embedder or vector-store code — those are later tasks; this task only widens the error vocabulary they will use.
- Keep `VektorError::State` reserved for SQLite/`HashStore` state errors so storage-vs-state stays semantically distinct.

## Acceptance criteria

- [ ] `VektorError` has distinct variants for embedding, vector-store/storage, and network failures
- [ ] Each new variant has a clear, category-prefixed `Display` message and is constructible from a `String`
- [ ] `#[from]` is implemented for at least `reqwest::Error` (or a documented reason why it cannot be)
- [ ] No new crate dependencies are added
- [ ] `src/error.rs` tests cover `Display` and `matches!` for every new variant
- [ ] No embedder/storage/CLI code changes in this task

## Verification

```bash
cargo test error
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git diff --check
```

## Notes / open questions

- If a foreign error type (notably `ort::Error` or `lancedb::Error`) is not `Send + Sync + 'static`, `#[from]` will fail to compile. That is expected — fall back to `.map_err(|e| VektorError::Embedding(e.to_string()))` and note it here so 3.2/3.6 don't re-litigate the decision.
- This task deliberately does NOT add an `Http`/status-code-rich network type; keep it a `String` for now. Richer retry/status modeling lives inside the OpenAI backend (3.4) and downloader (3.11), which already track HTTP status locally.
