# Task 1.1 — main entrypoint

**Phase**: 1 — Skeleton
**Task ID**: 1.1
**PRD reference**: Section 12 Function 1.1 (`main()`)
**Roadmap stage**: Stage 2 / `v0.1.0`
**Effort estimate**: S (≤1h)
**Depends on**: 0.1 (Cargo.toml), 0.3 (pre-commit hooks)
**Blocks**: 1.2, 1.4

## Objective

Replace the trivial `println!("vektor")` from task 0.1 with a real binary entry point: parse CLI args via `clap`, initialize the tokio async runtime, set up tracing, dispatch to the appropriate subcommand. Subcommand bodies are still stubs at this stage — task 1.4 fills them in.

The success metric here is "the binary takes args and dispatches structurally," not "any subcommand does anything."

## Inputs (must exist before starting)

- `Cargo.toml` from task 0.1 with `clap`, `tokio`, `tracing` declared
- `src/main.rs` with the trivial stub from task 0.1
- Pre-commit hooks installed (task 0.3) so this work doesn't ship unformatted

## Outputs (must exist after completion)

- `src/main.rs` — entry point with `#[tokio::main]`, structured arg parsing, error handling via `anyhow::Result`
- Empty module declarations referenced from main (e.g., `mod config; mod error; mod cli;`) so subsequent tasks can fill them in

## Approach

1. Rewrite `src/main.rs`. At this stage `cli::run` is a no-arg stub; task 1.4 will change it to `cli::run(cli: Cli)`. For task 1.1, keep it no-arg so main compiles:
   ```rust
   use anyhow::Result;

   mod cli;     // task 1.4 fills this in
   mod config;  // task 1.3 fills this in
   mod error;   // task 1.2 fills this in

   #[tokio::main]
   async fn main() -> Result<()> {
       // Real implementation lands in task 1.4. For now, dispatch to the
       // no-arg stub in cli.rs. Task 1.4 will change main to:
       //   let cli = cli::Cli::parse();
       //   cli::run(cli).await
       cli::run().await
   }
   ```
2. Create the three module stubs. **Empty `.rs` files are valid Rust** — `mod foo;` referring to an empty `foo.rs` compiles fine. Do NOT add `pub fn placeholder() {}` — an unused public function in a binary's private module triggers `dead_code` under `clippy -D warnings`. Instead, give each stub a single module-level doc comment so the file is non-empty and self-documenting:
   ```bash
   cat > src/error.rs <<'EOF'
   //! Vektor error types. Populated by task 1.2.
   EOF
   cat > src/config.rs <<'EOF'
   //! Vektor configuration. Populated by task 1.3.
   EOF
   ```
   Doc comments are not "items" so clippy never flags them.
3. `src/cli.rs` is special — main calls into it, so it must define at least the symbol main references. Add a temporary, deliberately-`unused`-allowed stub:
   ```rust
   //! Vektor CLI. Populated by task 1.4.

   #[allow(dead_code)]
   pub async fn run() -> anyhow::Result<()> {
       Ok(())
   }
   ```
   `#[allow(dead_code)]` on the function is acceptable because the call site exists in main.rs (so it's reachable), but clippy can briefly disagree during partial compilation. Remove the attribute in task 1.4 once `run()` does real work.
4. Run `cargo check` and `cargo clippy -- -D warnings`. Both must pass.
5. Run the binary: `cargo run -- --help`. It should exit 0 (no help text yet; that arrives with clap in task 1.4).

## Acceptance criteria

- [ ] `src/main.rs` uses `#[tokio::main]` and returns `anyhow::Result<()>`
- [ ] `src/main.rs` is <30 lines (per PRD Section 15 rule: every function readable in one screen)
- [ ] Module stubs exist: `src/cli.rs`, `src/config.rs`, `src/error.rs`
- [ ] `cargo check` exits 0
- [ ] `cargo clippy --all-targets -- -D warnings` exits 0
- [ ] `cargo run` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] No `unwrap()` in production code (use `?` operator)
- [ ] Pre-commit hook fires and passes on commit

## Verification

```bash
cargo check
cargo clippy --all-targets -- -D warnings
cargo run
cargo fmt --check
wc -l src/main.rs  # expect < 30 lines
grep -c "unwrap()" src/main.rs  # expect 0
```

## Notes / open questions

- **Why tokio's `#[tokio::main]` macro vs manual runtime construction?** The macro is fine for a binary's main. We never need to share the runtime with FFI or embed it elsewhere. Keep it simple.
- **Module stubs**: leaving `src/cli.rs` etc. truly empty makes Rust complain about "file not found" if `mod cli;` is declared without the file. The minimal placeholder fn keeps compilation clean.
- **`anyhow::Result` vs custom error type**: anyhow is fine at the top level. Custom error types via `thiserror` arrive in task 1.2 and are used by library code, not main.
- **Don't add real subcommand dispatch here**: that's task 1.4's job. Resist the temptation to "do it all in one task" — the dependency graph requires 1.1 → 1.2 → 1.3 → 1.4 sequencing because each task introduces concepts the next builds on.

## Commit

```
feat(main): 1.1 — real binary entrypoint with tokio + module stubs

Replaces the println!("vektor") stub from task 0.1. main is now
#[tokio::main] returning anyhow::Result<()>, dispatching to
cli::run() which itself is a stub until task 1.4. Empty module
files cli.rs, config.rs, error.rs created so subsequent tasks have
a place to write into without rewriting main.

Closes docs/plans/initial/phase-1-skeleton/01-main-entrypoint.md
```
