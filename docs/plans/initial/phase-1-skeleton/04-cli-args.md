# Task 1.4 — CLI args (clap derive subcommands)

**Phase**: 1 — Skeleton
**Task ID**: 1.4
**PRD reference**: Section 12 Function 1.1 (CLI args via clap), VEKTOR_ROADMAP.md Stage 2 ("vektor index" and "vektor serve" modes)
**Roadmap stage**: Stage 2 / `v0.1.0`
**Effort estimate**: M (1–4h)
**Depends on**: 1.1, 1.3
**Blocks**: 1.5

## Objective

Define the CLI surface via `clap`'s derive macro. v0.1.0 exposes 3 top-level subcommands (`index`, `serve`, `models`) and wires each to a stub function that returns `VektorError::NotImplemented`. The point is to lock down the CLI shape now so subsequent phases just fill in stub bodies — no breaking CLI changes during Phase 2/3/4.

## Inputs (must exist before starting)

- `src/cli.rs` exists with the temporary `run()` stub from task 1.1
- `Config` struct from task 1.3
- `Cargo.toml` declares `clap = { version = "4.6", features = ["derive"] }`

## Outputs (must exist after completion)

- `src/cli.rs` with the full clap-derive CLI definition + dispatcher
- 3 subcommand handler stubs returning `VektorError::NotImplemented`
- `vektor --help` produces useful help text per subcommand

## Approach

1. Define the CLI struct in `src/cli.rs`:
   ```rust
   use clap::{Parser, Subcommand};

   #[derive(Parser, Debug)]
   #[command(name = "vektor", version, about, long_about = None)]
   pub struct Cli {
       /// Path to config file (default: ~/.vektor/config.toml)
       #[arg(short, long, global = true)]
       pub config: Option<std::path::PathBuf>,

       /// Increase verbosity (-v for INFO, -vv for DEBUG, -vvv for TRACE)
       #[arg(short, long, global = true, action = clap::ArgAction::Count)]
       pub verbose: u8,

       #[command(subcommand)]
       pub command: Command,
   }

   #[derive(Subcommand, Debug)]
   pub enum Command {
       /// Index a codebase for later search and context assembly
       Index(IndexArgs),

       /// Start the MCP server for AI coding agents
       Serve(ServeArgs),

       /// Manage local ONNX models
       Models {
           #[command(subcommand)]
           action: ModelsAction,
       },
   }
   ```

2. Define each subcommand's args struct:
   ```rust
   #[derive(clap::Args, Debug)]
   pub struct IndexArgs {
       /// Path to the codebase to index
       pub path: std::path::PathBuf,

       /// Force a full re-index, ignoring hashes
       #[arg(long)]
       pub force: bool,

       /// Print chunks instead of indexing (debug aid for Phase 2)
       #[arg(long)]
       pub dump_chunks: bool,
   }

   #[derive(clap::Args, Debug)]
   pub struct ServeArgs {
       /// Transport mode: stdio (default) or sse
       #[arg(long, default_value = "stdio")]
       pub transport: String,
   }

   #[derive(Subcommand, Debug)]
   pub enum ModelsAction {
       /// Download the configured ONNX model
       Download {
           #[arg(long)]
           lite: bool,
       },
   }
   ```

3. Dispatcher:
   ```rust
   pub async fn run() -> crate::error::Result<()> {
       let cli = Cli::parse();
       let _config = crate::config::Config::load()?;  // task 1.3 wire-up

       match cli.command {
           Command::Index(_args) => Err(crate::error::VektorError::NotImplemented(
               "vektor index (Phase 2)",
           )),
           Command::Serve(_args) => Err(crate::error::VektorError::NotImplemented(
               "vektor serve (Phase 1 task 1.6 partial; Phase 4 full)",
           )),
           Command::Models { action: ModelsAction::Download { .. } } => Err(
               crate::error::VektorError::NotImplemented("vektor models download (Phase 3)"),
           ),
       }
   }
   ```

4. Test:
   - `vektor --help` lists all 3 subcommands
   - `vektor index --help` shows `path`, `--force`, `--dump-chunks`
   - `vektor index /tmp` exits with `error: not implemented: vektor index (Phase 2)` and exit code != 0
   - `vektor --version` prints the Cargo.toml version

5. Update `src/main.rs` to use the new `run()` signature (already wired in task 1.1).

## Acceptance criteria

- [ ] `Cli` struct uses `#[derive(Parser)]` with `version` and `about` attributes
- [ ] 3 subcommands present: `index`, `serve`, `models` (with `download` action)
- [ ] Global `--config` and `--verbose` flags work on every subcommand
- [ ] Each subcommand handler currently returns `VektorError::NotImplemented` with a phase reference
- [ ] `vektor --help` exits 0 and lists subcommands
- [ ] `vektor --version` matches `Cargo.toml`'s `version = "0.1.0"`
- [ ] Each subcommand has its own `--help` showing args
- [ ] `cargo test cli::tests` passes (snapshot tests for help output via `insta` optional but recommended)
- [ ] `cargo clippy --all-targets -- -D warnings` clean

## Verification

```bash
cargo build
./target/debug/vektor --help              # exits 0, shows subcommands
./target/debug/vektor --version           # prints "vektor 0.1.0"
./target/debug/vektor index --help        # shows path / --force / --dump-chunks
./target/debug/vektor serve --help        # shows --transport
./target/debug/vektor models download --help

# Stub exits non-zero with NotImplemented
./target/debug/vektor index /tmp 2>&1 | grep -q "not implemented"
[ $? -eq 0 ] || { echo "FAIL: index stub should return NotImplemented"; exit 1; }
```

## Notes / open questions

- **Why declare all subcommands now, not just `serve`?** Because the CLI surface is part of the public contract. Adding `vektor models` in Phase 3 means changing v0.1's CLI, which is technically a breaking change. Better to declare them all at v0.1 with stubs.
- **`vektor init` is NOT in this list**: per task allocation, `vektor init` ships at task 6.1. Adding it here is scope creep. Leave it for Phase 6.
- **Snapshot testing**: `insta` is a great tool for asserting help-text output doesn't drift. Optional for v0.1.0; if you add it, also add the `insta-cli` to dev-dependencies for `cargo insta accept`.
- **`run()` returning `VektorError` not `anyhow::Error`**: main wraps the error via `?`. Either ergonomics works; choose `VektorError` here so library callers can match on `NotImplemented` if needed.
- **`global = true` on `--config`/`--verbose`**: this means they work on any subcommand without re-declaration. Per clap's docs, global args can have edge cases with completion generation — verify completion works in task 6.2 if you add completions.

## Commit

```
feat(cli): 1.4 — clap derive subcommands: index / serve / models

Locks down the v0.1.0 CLI surface. Three top-level subcommands with
their argument structs declared; all handlers stub to
VektorError::NotImplemented with a phase reference indicating where
they get implemented. Global --config and --verbose flags work on
every subcommand.

vektor --help, vektor --version, and per-subcommand --help all work.
Adding new subcommands later is non-breaking; adding new required
args to existing subcommands would be — design accordingly.

Closes docs/plans/initial/phase-1-skeleton/04-cli-args.md
```
