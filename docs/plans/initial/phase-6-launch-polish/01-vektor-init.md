# Task 6.1 — `vektor init` MCP-config writer

## Objective

Add `vektor init` so a fresh install can register the local MCP server with supported coding agents without manual config editing.

## Scope

- Detect and update Claude Code, Cursor, and Codex CLI MCP config locations.
- Add `--agent`, `--force`, and `--dry-run`.
- Merge idempotently and preserve unrelated config.
- Refuse to overwrite an existing `vektor` entry unless `--force` is set.

## Acceptance Criteria

- [x] Claude Code config is written to `~/.claude.json`.
- [x] Cursor config is written to `~/.cursor/mcp.json`.
- [x] Codex CLI config is written to `~/.codex/config.toml`.
- [x] `--dry-run` prints the planned change without writing.
- [x] Existing unrelated config survives byte-for-byte where practical.
- [x] Existing `vektor` entries are protected without `--force`.

## Verification

```bash
cargo test init
cargo test mcp_config
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```
