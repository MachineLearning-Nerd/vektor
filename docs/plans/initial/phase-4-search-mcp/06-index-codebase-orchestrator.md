# Task 4.6 — Extend `index_codebase` orchestrator to also populate Tantivy

**Phase**: 4 — Search + MCP
**Task ID**: 4.6
**PRD reference**: Section 4.3 (Data Flow), Section 4.5 (Delete-Then-Insert), Section 12 Week 4 Function 4.6
**Roadmap stage**: Stage 2 (interim — no release tag)
**Effort estimate**: L
**Depends on**: 3.7c, 4.2
**Blocks**: 4.7

## Objective

Make the indexing pipeline build **both** stores in one pass. Phase 3 task 3.7c
already shipped a real, vector-only `index_codebase` MCP handler backed by the
shared `crate::cli::index_path` / `index_path_with_embedder` core. **This task
EXTENDS that existing core** to also write the Tantivy BM25 index — it does
**not** rebuild a new orchestrator or duplicate the discover→hash→chunk→embed
loop. After 4.6, one `vektor index <repo>` (and one `index_codebase` MCP call)
produces a queryable LanceDB table AND a committed Tantivy index.

## Inputs (must exist before starting)

- The Phase 3 shared index core: `crate::cli::index_path` (production) and
  `crate::cli::index_path_with_embedder` (test seam) — both already exist and are
  `pub(crate)`. The MCP handler `handle_index_codebase`
  (`src/mcp/handlers.rs`) already calls `index_path`.
- `TextIndex` with `new` / `add_chunks` / `commit` / `delete_by_file` (4.1, 4.2).
- The existing per-file flow already computes chunks + chunk ids + reuse decisions
  and calls `VectorStore` delete/insert (PRD §4.5). Tantivy writes hook into the
  SAME per-file flow.

## Outputs (must exist after completion)

- The shared index core opens a `TextIndex` for the project alongside the
  `VectorStore`, and for each processed file: `TextIndex::delete_by_file(rel_path)`
  then `TextIndex::add_chunks(...)` for that file's chunks — mirroring the
  LanceDB delete-then-insert (PRD §4.5 steps 2 + 5, "delete_by_file on Tantivy →
  insert new Tantivy entries").
- A single `TextIndex::commit()` at the END of the pass (PRD §4.3 "batch Tantivy
  commit, once per window — not per file").
- `index_depth = "deep"` on every Tantivy doc (ShallowIndexer is Phase 5).
- `IndexStats` is unchanged or additively extended; the MCP success JSON still
  returns the Phase 3 fields (`files`, `changed`, ..., `skipped_secrets`). Adding
  a Tantivy doc count is acceptable (additive).
- Secret-skipped chunks are excluded from Tantivy too (a skipped chunk must not
  be searchable by keyword any more than by vector).

## Approach

- Thread a `TextIndex` through `index_path_with_embedder` next to the
  `VectorStore` (constructor opens both stores under the same project data dir).
- In the per-file branch where chunks are inserted into LanceDB, also
  `delete_by_file` + `add_chunks` on the Tantivy index using the SAME `chunk_id`s
  (so cross-store join holds). Reuse the secret-filtered chunk set — do not
  re-run discovery or chunking.
- Commit Tantivy once after the file loop completes (and after the final
  LanceDB writes), so a crash mid-pass leaves HashStore `pending` and recovery
  re-runs cleanly (PRD §4.5 crash semantics).
- Keep the MCP handler thin: it still just calls the shared core; the core now
  does both stores. The handler's JSON contract is unchanged (additive at most).

## Acceptance criteria

- [ ] One `vektor index <repo>` run builds BOTH the LanceDB table AND a committed
      Tantivy index (phase exit criterion).
- [ ] The `index_codebase` MCP tool builds both stores via the same shared core —
      no second orchestrator, no duplicated loop (the Phase 3 handler is reused).
- [ ] Tantivy `chunk_id`s equal LanceDB `id`s for the same chunks (join holds).
- [ ] Re-indexing an unchanged repo does not duplicate Tantivy docs; a one-function
      edit updates only that file's Tantivy docs (delete_by_file scoping).
- [ ] Secret-skipped chunks appear in NEITHER store.
- [ ] Tantivy commit happens once per pass, not per file.
- [ ] No regression on Phase 2/3 tests; `IndexStats` JSON contract preserved
      (additive only).
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test index_cli            # existing Phase 3 tests still green
cargo test mcp::handlers        # index_codebase handler still returns real stats
cargo test text_index           # Tantivy writes via the orchestrator
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
# Model-gated end-to-end (manual, after `vektor models download`):
#   ./target/debug/vektor index <repo> && verify both ~/.vektor/<proj>/lance and .../tantivy exist
```

## Notes / open questions

- **This is an EXTENSION, not a rewrite.** The vector-only `index_codebase`
  handler shipped in Phase 3 (commits `928321e` + `afb1fb9`). 4.6 adds the Tantivy
  leg to the shared core both the CLI and the MCP handler already call.
- Keep the test seam (`index_path_with_embedder`) usable with a fake embedder so
  orchestrator tests don't need a model — extend it to also accept/observe the
  `TextIndex` so tests can assert both stores were written.
- Three commit semantics (LanceDB, Tantivy, SQLite HashStore) — order writes so
  HashStore is the source of truth on crash recovery (PRD §4.7). Chaos tests for
  this are Stage 4 (B5), not Phase 4.
