# Task 4.7 — MCP server bootstrap: real handler dispatch

**Phase**: 4 — Search + MCP
**Task ID**: 4.7
**PRD reference**: Section 9 (MCP server), Section 12 Week 4 Function 4.7
**Roadmap stage**: Stage 2 (interim — no release tag)
**Effort estimate**: M
**Depends on**: 1.6, 4.5, 4.6
**Blocks**: 4.8

## Objective

Wire the MCP server (rmcp, bootstrapped in task 1.6) so its tool handlers
dispatch into real engine functions instead of no-op stubs. `index_codebase` is
already real (Phase 3, extended in 4.6); this task ensures the server holds /
constructs the shared state (`VectorStore` + `TextIndex` + `Embedder`) that
`search_code` (4.8) needs, and that the rmcp tool registration/schemas are intact.

## Inputs (must exist before starting)

- The rmcp 1.7 server bootstrap + tool declarations from task 1.6
  (`src/mcp/server.rs`, `src/mcp/schemas.rs`, `src/mcp/handlers.rs`).
- `search_hybrid` (4.5) and the extended `index_codebase` core (4.6).
- The existing handler signatures: `handle_index_codebase` is `async` and real;
  `handle_search_code` / `handle_get_context_for_prompt` are still no-op stubs.

## Outputs (must exist after completion)

- The server can construct/lazily-open the per-project engine state (embedder,
  `VectorStore`, `TextIndex`) needed to serve `search_code`. Decide between
  lazy-on-first-search vs eager-at-startup (see Notes).
- `handle_search_code` no longer returns the "not implemented yet" stub — it
  routes to `search_hybrid` (the actual response shaping is task 4.8, but the
  dispatch wiring + state plumbing land here).
- rmcp tool registration, names, and input schemas are unchanged from 1.6 (only
  the handler bodies/state change), per the phase README note. Update
  human-readable tool descriptions and server instructions so they no longer say
  the tools are `v0.1.0` no-ops; those strings are not the input-schema contract.
- Server still writes nothing to stdout (stdio is the MCP transport); all
  diagnostics via `tracing` to stderr.

## Approach

- Hold engine state on the rmcp server/tool struct (or an `Arc`-shared context),
  built from the default `Config`. Opening `VectorStore`/`TextIndex` requires a
  project path; `search_code` receives that path as a required arg (the 1.6 input
  schema requires `["query", "path"]`), so open the stores for the requested
  `path` — the same project dir the CLI indexes — rather than inferring a single
  implicit project.
- Keep `handle_index_codebase` as-is (it already builds its own embedder + store
  per call via the shared core); `search_code` needs read handles to the already
  built stores, so plumb those.
- Do not change the tool schema/registration surface (1.6 contract). Replace stub
  bodies with dispatch + (in 4.8) response shaping. Refresh only descriptions and
  instructions that currently advertise no-op behavior.

## Acceptance criteria

- [ ] `vektor serve` starts and advertises the same three tools with the same
      names/input schemas as task 1.6 (no schema regression).
- [ ] Tool descriptions and server instructions no longer advertise `v0.1.0`
      no-op behavior after the handlers are real.
- [ ] `index_codebase` over MCP still returns real stats (Phase 3 + 4.6 behavior).
- [ ] `search_code` over MCP dispatches into `search_hybrid` (returns real ranked
      results once 4.8 shapes the response — at 4.7, dispatch + state plumbing are
      proven by a test that reaches `search_hybrid`).
- [ ] Server emits nothing on stdout; logs go to stderr via tracing.
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test --test mcp_serve     # existing serve bootstrap test still green
cargo test mcp::server
cargo test mcp::handlers
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- **Lazy vs eager engine state**: opening `VectorStore`/`TextIndex` at startup
  gives lower first-query latency (and pairs with the Phase 5 warm-up, task 5.12)
  but fails fast if no index exists yet. Lazy-on-first-call is more forgiving for
  "serve before index." PRD §4.4 two-tier expects "serve, then index in
  background," so lean toward lazy open + a clear "index building / not indexed"
  response. Flag the decision in this file when made.
- The rmcp 1.7 handler signatures already exist (1.6); this task changes bodies +
  shared state, NOT the registration/schema surface.
- `get_context_for_prompt` dispatch is task 4.8 (basic, naive at Phase 4); leave
  it stubbed until then unless trivially wired alongside `search_code`.
