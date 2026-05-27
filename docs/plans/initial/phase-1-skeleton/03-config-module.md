# Task 1.3 — config module (TOML + env override)

**Phase**: 1 — Skeleton
**Task ID**: 1.3
**PRD reference**: Section 6.3 (Configuration TOML schema), Section 9 (env override convention `VEKTOR_*`)
**Roadmap stage**: Stage 2 / `v0.1.0`
**Effort estimate**: M (1–4h)
**Depends on**: 1.2
**Blocks**: 1.4

## Objective

Define the `Config` struct that mirrors the TOML schema in PRD Section 6.3, load it from `~/.vektor/config.toml` if present (or use defaults), and apply environment-variable overrides per the `VEKTOR_*` convention.

Precedence (lowest to highest): defaults → file → env vars → CLI args (CLI overrides applied in task 1.4).

## Inputs (must exist before starting)

- `src/config.rs` exists as an empty stub from task 1.1
- `src/error.rs` has `VektorError::Config(String)` variant from task 1.2
- `Cargo.toml` declares `config = "0.15"`, `toml = "1"`, `serde = { features = ["derive"] }`, `dirs = "6"`

## Outputs (must exist after completion)

- `src/config.rs` defines `Config` struct with all fields from PRD Section 6.3 TOML schema
- `Config::load() -> Result<Config>` reads from file + env + falls back to defaults
- Unit tests covering defaults, file load, env override

## Approach

1. Define the `Config` struct mirroring PRD Section 6.3:
   ```rust
   use serde::{Deserialize, Serialize};

   #[derive(Debug, Clone, Serialize, Deserialize)]
   #[serde(default)]
   pub struct Config {
       pub embedding: EmbeddingConfig,
       pub index: IndexConfig,
       pub watcher: WatcherConfig,
       pub server: ServerConfig,
   }
   ```
   Plus one struct per section. Each field has a `#[serde(default = "default_X")]` or relies on `Default` impl.

2. Implement `Default` for every section so `Config::default()` produces the values from PRD Section 6.3 (`backend = "onnx"`, `chunk_max_lines = 200`, etc.).

3. Implement `Config::load()`:
   ```rust
   impl Config {
       pub fn load() -> crate::error::Result<Self> {
           let mut builder = config::Config::builder()
               .add_source(config::Config::try_from(&Config::default())?);

           // 1. file source (optional — present if ~/.vektor/config.toml exists)
           if let Some(home) = dirs::home_dir() {
               let path = home.join(".vektor").join("config.toml");
               if path.exists() {
                   builder = builder.add_source(config::File::from(path));
               }
           }

           // 2. env override: VEKTOR_FOO_BAR = X overrides foo.bar
           builder = builder.add_source(
               config::Environment::with_prefix("VEKTOR")
                   .separator("_")
                   .convert_case(config::Case::Snake),
           );

           builder
               .build()
               .and_then(|c| c.try_deserialize::<Config>())
               .map_err(|e| crate::error::VektorError::Config(e.to_string()))
       }
   }
   ```

4. Tests:
   - `test_default_config` — `Config::default()` matches PRD-section-6.3 values
   - `test_load_from_file` — write a temp TOML file with `[embedding] backend = "openai"`, verify it loads
   - `test_env_override` — set `VEKTOR_EMBEDDING_BACKEND=ollama` via `std::env::set_var`, verify it overrides the file value

5. The `config` crate v0.15 has API changes from v0.14. Read [docs.rs/config/0.15](https://docs.rs/config/0.15) for current `Config::builder()` and `Environment` API.

## Acceptance criteria

- [ ] `Config` struct has all 4 sections per PRD Section 6.3: `embedding`, `index`, `watcher`, `server`
- [ ] Each section has `Default` impl matching PRD-Section-6.3 default values
- [ ] `Config::load()` follows precedence: defaults → `~/.vektor/config.toml` → `VEKTOR_*` env vars
- [ ] Missing config file is NOT an error — defaults are used
- [ ] Malformed config file IS an error — returns `VektorError::Config`
- [ ] At least 3 unit tests covering defaults / file / env-override
- [ ] `cargo test config::tests` passes

## Verification

```bash
cargo test config::tests
cargo clippy -- -D warnings

# Sanity: print default config as TOML
cargo run --example print_default_config 2>/dev/null || echo "no example yet — fine"

# Env override sanity
VEKTOR_EMBEDDING_BACKEND=ollama cargo test config::tests::test_env_override
```

## Notes / open questions

- **`config` crate vs hand-rolled**: the `config` crate handles defaults + file + env merging with proper precedence. Hand-rolling this is a footgun (most "simple" implementations forget about case-conversion for env vars). Stick with the crate.
- **`figment` alternative**: figment is also good. We standardize on `config` because it's already in PRD Section 11. Don't introduce a new dep.
- **Env var separator**: `VEKTOR_EMBEDDING_BACKEND` → `embedding.backend`. The `config` crate's `separator("_")` + `convert_case(Snake)` handles this. Verify by reading the crate's docs — API has churned between versions.
- **Don't load on every call**: `Config::load()` should be called once in main (task 1.4) and the result passed around. Caching globally via `OnceCell` is unnecessary at v0.1.0.
- **Path expansion**: `dirs::home_dir()` returns the OS-conventional home. Don't manually expand `~`. Don't read `$HOME` directly.

## Commit

```
feat(config): 1.3 — TOML config + env-var override via config crate

Mirrors PRD Section 6.3 TOML schema as Rust structs with serde
derive + Default impls. Config::load() merges defaults → ~/.vektor/
config.toml (optional) → VEKTOR_* env vars with snake-case
conversion. Missing config file uses defaults; malformed file
returns VektorError::Config.

3 unit tests: defaults match PRD, file load, env override.

Closes docs/plans/initial/phase-1-skeleton/03-config-module.md
```
