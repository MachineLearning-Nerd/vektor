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
2. **Module visibility**: task 1.1's `src/main.rs` declares `mod error;` (private). That's correct for a binary crate — `pub mod` is only meaningful when something outside the crate consumes the module. Do NOT change to `pub mod`; it adds no value and is its own clippy nit. What matters for dead_code is that something in the crate **uses** `VektorError`/`Result` — see step below about the clippy gate.
3. **Do NOT re-export `Result` at the crate root.** Task 1.1's `src/main.rs` already does `use anyhow::Result;` for top-level propagation. Adding `pub use error::{Result, VektorError};` in main.rs would collide with the anyhow import (E0252: "the name `Result` is defined multiple times"). The two-layer pattern: library modules refer to `crate::error::Result` explicitly (or `use crate::error::Result;` locally); main keeps `anyhow::Result` for top-level `?`-into-anyhow propagation. If you ever need `VektorError` in main, import it as a single named item: `use crate::error::VektorError;` — that's collision-free.
4. **Add a placeholder use in `src/main.rs`** so the bin target's dead_code analyzer sees `VektorError` and `Result` as used. Without this, `cargo clippy --all-targets -- -D warnings` fails on the bin target even with all the unit tests in step 5 (test-target references don't propagate to bin-target dead_code analysis). The placeholder is a single line in main:
   ```rust
   // src/main.rs — between mod declarations and #[tokio::main]
   #[tokio::main]
   async fn main() -> anyhow::Result<()> {
       // task 1.2 placeholder use — bin target dead_code analyzer needs to see
       // VektorError and Result as referenced. Task 1.3 replaces this line with
       // `let _config = config::Config::load(None)?;` which propagates VektorError
       // naturally via the ? operator.
       let _placeholder: error::Result<()> = Ok(());

       cli::run().await
   }
   ```
   This pattern matches the round-3 fix that introduced the `Config::load(None)` placeholder for the same dead_code reason. Both placeholders disappear by task 1.4 when CLI dispatch starts genuinely consuming `VektorError`.

5. Verify `cargo check` still passes — no breaking changes to existing code.
6. Add unit tests that exercise each error variant's `Display` output and reference the `Result<T>` alias. (Note: tests alone are NOT sufficient for the bin-target clippy gate — see step 4's main.rs placeholder. Tests cover the test-target dead_code analysis; the placeholder covers the bin-target.)
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

       #[test]
       fn result_alias_uses_vektor_error() {
           let result: Result<()> = Err(VektorError::NotImplemented("test"));
           assert!(matches!(result, Err(VektorError::NotImplemented("test"))));
       }
   }
   ```

## Acceptance criteria

- [ ] `src/error.rs` defines `VektorError` enum with at minimum `Io`, `Config`, `NotImplemented`, `Mcp` variants
- [ ] `pub type Result<T> = std::result::Result<T, VektorError>;` exported at module level
- [ ] `#[from] std::io::Error` works (allows `?` on file ops)
- [ ] At least 2 unit tests exercising `Display` output, plus 1 unit test referencing the `Result<T>` alias
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --all-targets -- -D warnings` clean. **`--all-targets` runs clippy on each target separately** — the bin target and the test target each get their own dead_code pass. Tests-only references DO NOT silence dead_code in the bin target. To prevent the bin target from flagging `VektorError` and `Result`, task 1.2 ALSO updates `src/main.rs` with a placeholder use (see Approach step below). Same pattern as the `Config::load(None)` placeholder added in task 1.3.
- [ ] `src/main.rs` contains a placeholder use of `VektorError`/`Result` until task 1.3's `Config::load` propagation replaces it. Suggested form:
  ```rust
  // task 1.2 placeholder — task 1.3 replaces this with Config::load(None)? which propagates VektorError naturally
  let _placeholder: error::Result<()> = Ok(());
  ```

## Verification

```bash
cargo test error::tests
cargo clippy --all-targets -- -D warnings   # --all-targets compiles tests so VektorError and Result<T> uses count
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

Includes unit tests verifying Display output and the Result<T> alias.

Closes docs/plans/initial/phase-1-skeleton/02-error-module.md
```
