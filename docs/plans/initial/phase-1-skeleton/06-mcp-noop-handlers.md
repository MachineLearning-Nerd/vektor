# Task 1.6 — MCP no-op handlers (rmcp 1.7)

**Phase**: 1 — Skeleton
**Task ID**: 1.6
**PRD reference**: Section 9 (MCP Tools API surface), Section 12 Function 4.7 (MCP server bootstrap — partial v0.1 implementation)
**Roadmap stage**: Stage 2 / `v0.1.0`
**Effort estimate**: L (4–8h)
**Depends on**: 1.5
**Blocks**: 1.7

## Objective

Implement `vektor serve` with a real rmcp 1.7 server over stdio. Register the 3 primary v0.1 tools (`index_codebase`, `search_code`, `get_context_for_prompt`) with no-op handlers that return `{"status": "not implemented yet", "phase": "<phase-ref>"}`. The server must respond correctly to MCP protocol-level requests: `initialize`, `tools/list`, `tools/call`.

This is the **longest task in Phase 1**. Read rmcp 1.7's docs.rs page before starting; the API is substantially different from the 0.x examples in PRD Section 12.

## Inputs (must exist before starting)

- `Cli::Serve` variant with `--transport` arg from task 1.4
- `tracing` initialized to JSON format for serve (task 1.5)
- `Cargo.toml` has `rmcp = { version = "1.7", features = ["server", "transport-io"] }`

## Outputs (must exist after completion)

- `src/mcp/mod.rs` and `src/mcp/handlers.rs` (or `src/mcp.rs` if it fits in <300 lines)
- Server starts on stdio when `vektor serve --transport stdio` runs
- `tools/list` returns the 3 tools with proper JSON schemas
- `tools/call` for each tool returns the no-op response
- At least 1 integration test that spawns the server and sends a real MCP request

## Approach

1. **Read the rmcp 1.7 docs first.** Specifically:
   - [docs.rs/rmcp/1.7.0](https://docs.rs/rmcp/1.7.0) — top-level structure
   - The `server` module and its `Service`/`ServerHandler` traits
   - The `transport::io` module for stdio transport
   - At least one example that ships in the rmcp repo

2. Design the module layout:
   ```
   src/mcp/
   ├── mod.rs        — pub use; entry point fn start_stdio_server()
   ├── server.rs     — VektorServer struct implementing ServerHandler
   ├── handlers.rs   — handle_index_codebase, handle_search_code, handle_get_context_for_prompt
   └── schemas.rs    — JSON schemas for the 3 tools (input + output)
   ```

3. Implement `VektorServer`:
   ```rust
   pub struct VektorServer {
       config: Config,
   }

   impl ServerHandler for VektorServer {
       fn list_tools(&self, _req: ListToolsRequest) -> Result<ListToolsResult, ...> {
           Ok(ListToolsResult {
               tools: vec![
                   index_codebase_tool(),
                   search_code_tool(),
                   get_context_for_prompt_tool(),
               ],
           })
       }

       fn call_tool(&self, req: CallToolRequest) -> Result<CallToolResult, ...> {
           let response = match req.name.as_str() {
               "index_codebase" => handlers::handle_index_codebase(req.arguments),
               "search_code" => handlers::handle_search_code(req.arguments),
               "get_context_for_prompt" => handlers::handle_get_context_for_prompt(req.arguments),
               other => return Err(...),
           };
           Ok(CallToolResult::text(serde_json::to_string(&response)?))
       }
   }
   ```

   (The actual rmcp 1.7 API signatures will differ — verify against docs.rs.)

4. Each handler returns the no-op JSON:
   ```rust
   pub fn handle_index_codebase(_args: serde_json::Value) -> serde_json::Value {
       serde_json::json!({
           "status": "not implemented yet",
           "phase": "Phase 2 — Discovery + Chunking (v0.2.0)",
           "see": "docs/plans/initial/phase-2-discovery-chunking/README.md"
       })
   }
   ```

5. Tool schemas (`src/mcp/schemas.rs`):
   - For each tool, define **only `input_schema`** at v0.1.0 — matching PRD Section 9's request shapes. Do NOT advertise PRD Section 9's response shapes as `output_schema`. The PRD §9 output shapes describe the REAL handler responses (rich `context` / `chunks` / `metadata` payloads); at v0.1.0 every handler returns the no-op `{"status": "not implemented yet", "phase": "..."}` JSON, which would violate any output_schema we publish. Omit `output_schema` from the tool declarations at v0.1.0. **Phase 4 (when handlers do real work) adds the output_schema fields to match PRD §9.** If rmcp 1.7 requires an output_schema field, publish a minimal stub matching the no-op shape: `{"type": "object", "properties": {"status": {"type": "string"}, "phase": {"type": "string"}}}`.
   - Use `serde_json::json!` macro for simplicity at v0.1; switch to derive-based schemas (e.g., `schemars`) in Phase 4

6. Integration test (`tests/mcp_serve.rs`):
   - **Spawn `vektor serve --transport stdio` with an isolated HOME/USERPROFILE.** Once task 1.4 wires `Config::load(cli.config.clone())` into the dispatcher, the subprocess will walk `dirs::home_dir()` to find `~/.vektor/config.toml`. On any developer/CI machine with an existing personal config (especially a malformed or future-schema one), the subprocess fails to start before MCP messages flow. Set `HOME`/`USERPROFILE` to a fresh tempdir on the child process — same isolation pattern as the config unit tests in task 1.3:
     ```rust
     use std::process::{Command, Stdio};
     let fake_home = tempfile::tempdir().unwrap();
     let mut child = Command::new(env!("CARGO_BIN_EXE_vektor"))
         .arg("serve")
         .args(["--transport", "stdio"])
         .env("HOME", fake_home.path())
         .env("USERPROFILE", fake_home.path())  // Windows
         .stdin(Stdio::piped())
         .stdout(Stdio::piped())
         .stderr(Stdio::piped())
         .spawn()
         .expect("spawn vektor serve");
     ```
     Alternative: pass `--config /path/to/empty.toml` to the child, but the empty-file approach requires creating the file too. Isolating HOME is simpler and proves the subprocess truly runs against defaults.
   - Send an `initialize` request with **all three required fields** — rmcp 1.7's `InitializeRequestParams` requires `protocolVersion`, `capabilities`, AND `clientInfo`. Omitting `clientInfo` produces an invalid initialize that may be rejected before reaching `tools/list`:
     ```json
     {
       "jsonrpc": "2.0",
       "id": 1,
       "method": "initialize",
       "params": {
         "protocolVersion": "2024-11-05",
         "capabilities": {},
         "clientInfo": { "name": "vektor-integration-test", "version": "0.1.0" }
       }
     }
     ```
   - Read response from stdout
   - Send `{"jsonrpc":"2.0","id":2,"method":"tools/list"}` — verify 3 tools listed
   - Send a `tools/call` with **schema-valid arguments** so the test doesn't depend on rmcp's validation being lax:
     ```json
     {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"index_codebase","arguments":{"path":"/tmp/dummy"}}}
     ```
     The PRD §9 schema requires `path` for `index_codebase`. v0.1 handlers ignore the argument content and return the no-op JSON regardless — but the *schema* must validate so the call reaches the handler at all. Passing `{}` produces a validation failure depending on rmcp's strictness mode, which would make the test brittle. Same pattern for the other two tools when added:
     - `search_code` → `{"query": "test", "path": "/tmp/dummy"}`
     - `get_context_for_prompt` → `{"query": "test", "path": "/tmp/dummy", "token_budget": 8000}`

7. Wire `start_stdio_server` into `Command::Serve` handler in `src/cli.rs`.

## Acceptance criteria

- [ ] `vektor serve` starts and stays running (until SIGTERM or stdin EOF)
- [ ] `tools/list` returns exactly the 3 expected tool names
- [ ] Each `tools/call` returns the no-op JSON with `phase` field set
- [ ] `tools/call` with an unknown tool name returns an MCP error response (not a crash)
- [ ] Stdout contains **only** MCP protocol messages (no log noise) — verify by running serve with `-vvv` and checking stdout is parseable as JSON-RPC line-by-line
- [ ] Integration test in `tests/mcp_serve.rs` passes
- [ ] Server shuts down cleanly on stdin EOF or SIGTERM
- [ ] No `unwrap()` outside `#[cfg(test)]`
- [ ] `cargo clippy --all-targets -- -D warnings` clean

## Verification

```bash
# Spawn server and send a manual request
{
  echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"manual-test","version":"0.1.0"}}}'
  sleep 0.1
  echo '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
  sleep 0.1
} | ./target/debug/vektor serve --transport stdio | head -3

# Integration test
cargo test --test mcp_serve

# Stdout cleanliness
./target/debug/vektor -vvv serve > /tmp/stdout.txt 2> /tmp/stderr.txt &
PID=$!
sleep 1
kill $PID
# stdout should be empty (no requests sent yet); stderr should have log noise
[ -s /tmp/stdout.txt ] && echo "FAIL: stdout had data without a request" || echo "OK"
[ -s /tmp/stderr.txt ] && echo "OK: stderr has logs"
```

## Notes / open questions

- **rmcp 1.7 API churn**: the API differs from the 0.x examples in PRD Section 12. The PRD's Function 4.7 skeleton uses `RegisterTool` macros that may not exist in 1.7. The fix is in the executing task: read 1.7 docs, adapt the design. Do not regress to 0.x just because the PRD says so — the PRD's Risk-13 update already flagged this gap.
- **JSON schemas vs `schemars`**: at v0.1.0 we write JSON schemas by hand using **`serde_json::json!`** (the `schemars` crate is NOT a Cargo.toml dependency at v0.1.0). The Approach section above uses `serde_json::json!` consistently — follow that. In Phase 4 we may add `schemars` as a dep and use its derive macros so input types and schemas stay in sync. Not worth it yet.
- **Stdio vs SSE**: only stdio in v0.1.0. SSE transport adds auth concerns (PRD B2.3 — Phase 4 territory). Return `VektorError::NotImplemented` for `--transport sse`.
- **Async vs sync handlers**: rmcp 1.7 supports both. We use async (already in a tokio context per main); future handlers that hit LanceDB / ONNX need async.
- **Test isolation**: spawning a real `vektor serve` subprocess in tests is slow (1s per test). Keep integration tests minimal; unit-test the handlers directly.
- **Protocol version**: `2024-11-05` is the MCP protocol version. Verify against the MCP spec when this lands; if 2026 versions exist, use the latest stable.
- **Why XL effort estimate isn't on this**: the task is L because the rmcp surface is small (3 tools, no real work). It would be XL if we had to implement the embedder, store, etc. — those are downstream phases.

## Commit

```
feat(mcp): 1.6 — rmcp 1.7 stdio server with 3 no-op tool handlers

Implements vektor serve over stdio. Registers index_codebase,
search_code, and get_context_for_prompt with handlers that return
{"status": "not implemented yet", "phase": "..."} JSON. tools/list
returns proper schemas matching PRD Section 9. Unknown tool names
get proper MCP error responses instead of crashing.

Integration test in tests/mcp_serve.rs spawns the binary, sends
initialize + tools/list + tools/call requests, and asserts on
responses. Stdout reserved exclusively for MCP; logs to stderr.

Closes docs/plans/initial/phase-1-skeleton/06-mcp-noop-handlers.md
```
