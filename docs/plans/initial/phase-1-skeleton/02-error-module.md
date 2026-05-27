# Task 1.2 — error module (thiserror)

**Phase**: 1 — Skeleton
**Task ID**: 1.2
**PRD reference**: Section 15 (Implementation Rules — "use `?` and proper error types, no `.unwrap()` in production paths")
**Roadmap stage**: Stage 2 / `v0.1.0`
**Effort estimate**: S (≤1h)
**Depends on**: 1.1
**Blocks**: 1.3

## Objective

Define the project-wide error type `VektorError` using `thiserror`. Library-style modules (chunker, embedder, store, mcp) will return `Result<T, VektorError>`. The top-level binary continues to use `anyhow::Result` for ergonomic propagation.

This task establishes the **error contract** that every subsequent module will follow.

## Inputs (must exist before starting)

- `src/error.rs` exists as an empty stub from task 1.1
- `thiserror = "2"` declared in `Cargo.toml`

## Outputs (must exist after completion)

- `src/error.rs` populated with `VektorError` enum + `pub type Result<T> = std::result::Result<T, VektorError>;`
- A handful of variants seeded for known error categories (more will be added as Phase 2+ tasks discover them)

## Approach

1. Write `src/error.rs`:
   ```rust
   use thiserror::Error;

   pub type Result<T> = std::result::Result<T, VektorError>;

   #[derive(Debug, Error)]
   pub enum VektorError {
       #[error("IO error: {0}")]
       Io(#[from] std::io::Error),

       #[error("config error: {0}")]
       Config(String),

       #[error("not implemented: {0}")]
       NotImplemented(&'static str),

       #[error("MCP protocol error: {0}")]
       Mcp(String),
   }
   ```
2. Make the error type publicly exported by adding `pub mod error;` already done in task 1.1.
3. Re-export at crate root for convenience:
   ```rust
   // In src/main.rs or src/lib.rs (if/when extracted):
   pub use error::{Result, VektorError};
   ```
4. Verify `cargo check` still passes — no breaking changes to existing code.
5. Add a unit test that exercises each error variant's `Display` output:
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;

       #[test]
       fn display_io_error() {
           let e: VektorError = std::io::Error::new(std::io::ErrorKind::NotFound, "x").into();
           assert!(e.to_string().contains("IO error"));
       }

       #[test]
       fn display_config_error() {
           let e = VektorError::Config("bad value".into());
           assert_eq!(e.to_string(), "config error: bad value");
       }
   }
   ```

## Acceptance criteria

- [ ] `src/error.rs` defines `VektorError` enum with at minimum `Io`, `Config`, `NotImplemented`, `Mcp` variants
- [ ] `pub type Result<T> = std::result::Result<T, VektorError>;` exported at module level
- [ ] `#[from] std::io::Error` works (allows `?` on file ops)
- [ ] At least 2 unit tests exercising `Display` output
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy -- -D warnings` clean

## Verification

```bash
cargo test error::tests
cargo clippy -- -D warnings
grep -E "^pub (enum|type) " src/error.rs  # expect VektorError + Result
```

## Notes / open questions

- **Why both `anyhow` and `thiserror`?** Two-layer pattern: `thiserror` defines the structured library error type; `anyhow` is the catch-all at the binary's main. Library code propagates `VektorError` so callers (and tests) can match on variants. The binary doesn't care about specific variants — it just needs to print and exit.
- **Don't pre-declare every possible variant**: only the ones we actually `?`-into at this stage. Phase 2/3 tasks will add more (`Chunk(String)`, `Embed(String)`, `LanceDb(...)`, etc.) as they hit them. Premature variants are clutter.
- **`Source` errors with `#[from]`**: prefer `#[from]` over manual `From` impls. It's idiomatic thiserror and keeps the variants clean.
- **No `eyre` / `color-eyre`**: anyhow is sufficient. Adding eyre is a "nice colorful traceback" upgrade we don't need at v0.1.0.

## Commit

```
feat(error): 1.2 — define VektorError + Result via thiserror

Establishes the project-wide error type. Library modules return
Result<T, VektorError>; the binary's main keeps anyhow::Result for
top-level propagation. Starter variants: Io (#[from] std::io::Error),
Config, NotImplemented, Mcp. More variants added by Phase 2+ tasks
as they discover them.

Includes 2 unit tests verifying Display output.

Closes docs/plans/initial/phase-1-skeleton/02-error-module.md
```
