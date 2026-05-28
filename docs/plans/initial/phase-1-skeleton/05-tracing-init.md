# Task 1.5 — tracing init

**Phase**: 1 — Skeleton
**Task ID**: 1.5
**PRD reference**: Section 11 (`tracing` + `tracing-subscriber` declared deps)
**Roadmap stage**: Stage 2 / `v0.1.0`
**Effort estimate**: S (≤1h)
**Depends on**: 1.4
**Blocks**: 1.6

## Objective

Initialize structured logging via `tracing` + `tracing-subscriber`. The `--verbose` count flag from task 1.4 controls log level. Log output goes to stderr by default. The format is human-readable for `vektor index` and structured JSON for `vektor serve` (so MCP stdio's stdout stays clean for the protocol).

## Inputs (must exist before starting)

- `Cli::verbose` field from task 1.4 (count of `-v` flags)
- `Cargo.toml` declares `tracing = "0.1"` and `tracing-subscriber = { features = ["env-filter", "json"] }`

## Outputs (must exist after completion)

- `src/telemetry.rs` (new module) with `init(verbose: u8, format: Format) -> ()`
- `src/main.rs` calls `telemetry::init(...)` after `Cli::parse()` and before `cli::run()`

## Approach

0. **Cargo.toml prerequisite**: ensure `tracing-subscriber` has both `env-filter` AND `json` features. `fmt::layer().json()` (used below) is gated behind the `json` feature — without it the code below produces `error[E0599]: no method named "json" found for struct Layer`. If the `json` feature is missing, update the line in `Cargo.toml`:
   ```toml
   tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
   ```
   Also update PRD Section 11 Cargo.toml block to keep the canonical spec aligned. Run `cargo check` after the edit to regenerate `Cargo.lock`; commit the lockfile change with the task 1.5 commit.

1. Create `src/telemetry.rs`:
   ```rust
   use tracing_subscriber::{fmt, prelude::*, EnvFilter};

   pub enum Format {
       Pretty,
       Json,
   }

   pub fn init(verbose: u8, format: Format) {
       let default_level = match verbose {
           0 => "warn",
           1 => "info",
           2 => "debug",
           _ => "trace",
       };

       let filter = EnvFilter::try_from_env("VEKTOR_LOG")
           .unwrap_or_else(|_| EnvFilter::new(format!("vektor={default_level}")));

       let registry = tracing_subscriber::registry().with(filter);

       match format {
           Format::Pretty => {
               registry.with(fmt::layer().with_writer(std::io::stderr)).init();
           }
           Format::Json => {
               registry.with(fmt::layer().json().with_writer(std::io::stderr)).init();
           }
       }
   }
   ```

2. Add `mod telemetry;` to `src/main.rs` and call:
   ```rust
   let cli = Cli::parse();
   let format = match &cli.command {
       Command::Serve(_) => telemetry::Format::Json,
       _ => telemetry::Format::Pretty,
   };
   telemetry::init(cli.verbose, format);
   ```

3. In each subcommand stub, add a `tracing::info!("...")` line to demonstrate the logging works:
   ```rust
   Command::Index(args) => {
       tracing::info!(?args.path, "vektor index requested");
       Err(VektorError::NotImplemented("..."))
   }
   ```

4. Test:
   - `vektor index /tmp` (no -v) — no tracing output (level WARN, info not shown)
   - `vektor -v index /tmp` — info line visible
   - `vektor -vvv index /tmp` — trace level
   - `VEKTOR_LOG=debug vektor index /tmp` — env var wins
   - `vektor serve` — logs are JSON formatted, stdout untouched

## Acceptance criteria

- [ ] `src/telemetry.rs` exists with `init` function
- [ ] `tracing::info!` calls in subcommand stubs produce output at `-v` level
- [ ] No tracing output without `-v` flag at INFO level (default = WARN)
- [ ] `vektor serve` emits JSON log lines to stderr (NOT stdout — stdout is reserved for MCP)
- [ ] `VEKTOR_LOG=debug` env var overrides verbosity
- [ ] All log output goes to stderr (verify with `vektor index 2>/dev/null` showing no lines)
- [ ] `cargo test telemetry::tests` passes (test the format-selection logic, not the global subscriber)

## Verification

> **Verification commands MUST exit non-zero on failed checks.** The previous form used patterns like `cmd && echo FAIL || echo OK`, whose final exit status is `echo`'s — always 0 — so a CI runner sees "passed" even when FAIL is printed. The patterns below use explicit `if/then/exit 1` so any failed check propagates a non-zero exit, which an executor (CI, agent, human-with-`set -e`) can actually detect.

```bash
set -e   # belt-and-braces: bash exits on first unhandled failure

# Default: no INFO logs (level = WARN)
if ./target/debug/vektor index /tmp 2>&1 | grep -q "INFO"; then
    echo "FAIL: should be no INFO output at default level"
    exit 1
fi
echo "OK: default level suppresses INFO"

# -v: INFO shown
if ! ./target/debug/vektor -v index /tmp 2>&1 | grep -q "INFO"; then
    echo "FAIL: -v should produce INFO output"
    exit 1
fi
echo "OK: -v produces INFO"

# serve uses JSON on stderr
FIRST_STDERR_LINE=$(./target/debug/vektor -v serve 2>&1 1>/dev/null | head -1)
if ! echo "$FIRST_STDERR_LINE" | python3 -c "import sys, json; json.loads(sys.stdin.read())" >/dev/null 2>&1; then
    echo "FAIL: vektor serve stderr should be JSON; first line was: $FIRST_STDERR_LINE"
    exit 1
fi
echo "OK: vektor serve emits JSON on stderr"

# stdout untouched on serve (reserved for MCP)
./target/debug/vektor -v serve > /tmp/stdout.txt 2>/dev/null
if [ -s /tmp/stdout.txt ]; then
    echo "FAIL: vektor serve stdout should be empty at startup; got:"
    head -5 /tmp/stdout.txt
    exit 1
fi
echo "OK: vektor serve stdout clean at startup"

rm -f /tmp/stdout.txt
echo "OK: all tracing verification checks passed"
```

## Notes / open questions

- **Why stderr-only?** MCP stdio mode owns stdout. Any tracing output on stdout would corrupt the JSON-RPC protocol. Hard rule: **never** write logs to stdout from a `vektor serve` process.
- **JSON format for `serve`**: lets future log-aggregation tools parse `vektor serve` output cleanly without regex. Adds ~10% verbosity but worth it.
- **`VEKTOR_LOG`**: standard env var name following `RUST_LOG` convention. Per PRD Section 9 env-var convention `VEKTOR_*` prefix.
- **Don't bring in `tracing-bunyan-formatter` or other formats**: keep dep surface minimal at v0.1. Pretty + JSON via `fmt::layer().json()` is enough.
- **Log level filter for our crate vs the world**: `EnvFilter::new("vektor=info")` filters by crate name. If we want to silence noisy upstream crates (e.g., `tantivy=warn`), add that later in a config file.

## Commit

```
feat(telemetry): 1.5 — tracing-subscriber init with verbose mapping + JSON for serve

New module src/telemetry.rs initializes tracing-subscriber. CLI's
-v count maps to log level (0=warn, 1=info, 2=debug, 3+=trace).
VEKTOR_LOG env var overrides. Pretty format for one-shot commands;
JSON format for vektor serve (stdout is reserved for MCP, all logs
go to stderr).

Subcommand stubs gain tracing::info! lines proving the wiring.

Closes docs/plans/initial/phase-1-skeleton/05-tracing-init.md
```
