# Task 5.8 — `IndexStatusTracker` — shared two-tier index phase

**Phase**: 5 — Context Assembly
**Task ID**: 5.8
**PRD reference**: Section 4.4 (Two-Tier Indexing Strategy)
**Roadmap stage**: Stage 2 / v0.4.0
**Effort estimate**: S
**Depends on**: 5.7
**Blocks**: 5.13

## Objective

Give the MCP server a single source of truth for how complete the index is, so
every handler can answer with the right tier and the right caveat. PRD §4.4
defines three answer qualities: while nothing is indexed, tools should return
empty results with a "building" message; after the shallow pass (5.7), they
return keyword/shallow results tagged `"partial"`; after the deep pass (4.6),
they return full hybrid results tagged `"full"`. This task introduces the shared
`IndexStatusTracker` the handlers consult before and during search, and which
the shallow/deep indexers advance as each tier completes.

## Inputs (must exist before starting)

- `ShallowIndexer::build` (5.7) — completion of the shallow pass is the
  `Building → Partial` transition trigger.
- The deep index core (4.6, `crate::cli::index_path*`) — completion of the deep
  pass is the `Partial → Full` transition trigger.
- The MCP handlers (`src/mcp/handlers.rs`) — they already emit an
  `index_status` string into responses (see `metadata.index_status`); today it
  is derived per call. This task replaces the ad hoc derivation with the shared
  tracker.

## Outputs (must exist after completion)

- An `IndexPhase` enum: `Building`, `Partial`, `Full`.
- `IndexStatusTracker` backed by `Arc<RwLock<IndexPhase>>`, cloneable and shared
  across the MCP server's handler state, with:
  - a constructor that starts at `Building` (or "none"→`Building` at index
    start),
  - `mark_partial()` / `mark_full()` (called by the shallow / deep passes),
  - a read accessor returning the current `IndexPhase`,
  - an `as_str()`/serialization mapping to the exact response strings
    `"building"`, `"partial"`, `"full"`.
- Handler integration: each search/context handler reads the tracker and shapes
  its response accordingly:
  - `Building` → empty results + a "index building" message (no model load),
  - `Partial` → keyword/shallow results only (no semantic/hybrid),
  - `Full` → hybrid results.
  The chosen string lands in `metadata.index_status` (and, for context, feeds
  `budget_gap_reason = IndexIncomplete` when not `Full` — wired in 5.5/5.6).
- Legal transitions only: `Building → Partial → Full`. Never skip backward; a
  re-index may reset to `Building` then advance again.

## Approach

- Define `IndexPhase` + `IndexStatusTracker` in a small module (e.g.
  `src/index_status.rs` or under `src/mcp/`). Keep it dependency-free — just
  `std::sync::{Arc, RwLock}`.
- Hold one `IndexStatusTracker` clone on the rmcp server/handler state
  (alongside the engine state plumbed in 4.7) so all handlers share it.
- The shallow pass calls `mark_partial()` on success; the deep pass calls
  `mark_full()` on success. A fresh `vektor index` / re-index resets to
  `Building` before the shallow pass starts.
- Replace the per-call `index_status` derivation in `src/mcp/handlers.rs` with a
  read of the tracker. Preserve the existing response shape — the string values
  `"building"`/`"partial"`/`"full"` are the contract the tests already assert
  (e.g. `response["metadata"]["index_status"]`).
- Keep lock holds short: read the phase, drop the guard, then do work. Never
  hold the `RwLock` across an `.await`.

## Acceptance criteria

- [ ] `IndexPhase` has exactly `Building`, `Partial`, `Full`, mapping to
      `"building"`, `"partial"`, `"full"`.
- [ ] `IndexStatusTracker` is `Arc<RwLock<IndexPhase>>`-backed, `Clone`, and
      shareable across handlers.
- [ ] A fresh tracker reads `Building`; `mark_partial()` then reads `Partial`;
      `mark_full()` then reads `Full`.
- [ ] Transitions are forward-only within a pass (`Building→Partial→Full`); a
      re-index resets to `Building` then re-advances.
- [ ] With phase `Building`, a search handler returns empty results + an
      index-building message and does NOT build an embedder/load a model.
- [ ] With phase `Partial`, search returns keyword/shallow results and tags
      `index_status: "partial"`; with `Full`, it returns hybrid results tagged
      `"full"`.
- [ ] The shallow pass advances the shared tracker to `Partial`; the deep pass
      advances it to `Full` (proven via a handler/integration test).
- [ ] No `RwLock` guard is held across an `.await`.
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test index_status                  # tracker transitions + string mapping
cargo test mcp::handlers                  # handlers consult the tracker; status strings preserved
cargo test mcp::handlers::tests::keyword_search_does_not_build_embedder
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- The handlers already emit `metadata.index_status` and gate model loads (the
  Phase 4 `keyword_search_does_not_build_embedder` test exists). This task makes
  the status a *shared, indexer-driven* value rather than a per-call inference —
  do not change the string contract or the no-model-on-keyword behavior.
- Open question: where does the tracker live — a standalone `src/index_status.rs`
  or inside the MCP server state module? It is consumed by handlers and written
  by indexers, so a top-level module avoids an `mcp → cli` import cycle. Decide
  and note here.
- `index_status` also appears in `search_code` responses, not only
  `get_context_for_prompt`; route all three tools through the same tracker.
