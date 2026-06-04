# Task 5.8 — `IndexStatusTracker` — project-scoped two-tier index phase

**Phase**: 5 — Context Assembly
**Task ID**: 5.8
**PRD reference**: Section 4.4 (Two-Tier Indexing Strategy)
**Roadmap stage**: Stage 2 / v0.4.0
**Effort estimate**: S
**Depends on**: 5.7
**Blocks**: 5.13

## Objective

Give the MCP server a shared, project-scoped index phase source of truth so handlers can
return the right caveat for each project independently. PRD §4.4 expects three behaviors:
while nothing is indexed, handlers return empty results with a building message; after
shallow indexing they return keyword/shallow results tagged `"partial"`; after deep indexing
they return hybrid results tagged `"full"`.

This task introduces a tracker keyed by `project_hash` (or canonical project root), initialized
from on-disk index health at process start, and advanced by the shallow/deep indexers.

## Inputs (must exist before starting)

- `ShallowIndexer::build` (5.7) — completion of the shallow pass triggers the per-project
  `Building → Partial` transition.
- The deep index core (4.6, `crate::cli::index_path*`) — completion of the deep pass triggers
  the per-project `Partial → Full` transition.
- On-disk index state available at startup so status is reconstructed without waiting for
  reindex completion:
  - project text-index searchable marker in `text_index.rs` and
  - vector store metadata (`vector_meta.json`) indicating last completed full ANN index.
- The MCP handlers (`src/mcp/handlers.rs`) that currently derive `index_status` per call.

## Outputs (must exist after completion)

- `IndexPhase` enum: `Building`, `Partial`, `Full`.
- `IndexStatusTracker` that is scoped by project key and internally backed by
  `Arc<RwLock<HashMap<String, IndexPhase>>>` (or equivalent nested map), cloneable and
  shared across server state:
  - `new(configured_projects: impl IntoIterator<ProjectKey>) -> Self` (optional bootstrap form)
    that seeds each known project from on-disk health;
  - `set_phase(project_key: &str, phase: IndexPhase)`;
  - `mark_building(project_key: &str)`;
  - `mark_partial(project_key: &str)`;
  - `mark_full(project_key: &str)`;
  - `status(project_key: &str) -> IndexPhase` (defaults to `Building` if unknown);
  - `as_str(phase) -> "building" | "partial" | "full"`.
- A canonical `project_key` derivation strategy (`project_hash` from `state::project_dir` or
  normalized root path) that matches cache/handler expectations.
- Handler integration for each search/context path to read the tracker by project key:
  - `Building` → empty results + an "index building" message (no model/embedding load).
  - `Partial` → keyword/shallow results only.
  - `Full` → hybrid results.
  `index_status` lands in `metadata.index_status`; context handler also maps `index_status !=
  "full"` to `budget_gap_reason = IndexIncomplete`.

## Approach

- Define `IndexPhase` + tracker in a dedicated module (e.g. `src/index_status.rs`) so it can be
  shared by `src/mcp` and CLI indexing flow without import cycles.
- Store one shared tracker instance on the RMCP server state; handlers read by key, indexers write
  by key.
- On process startup, seed the tracker from disk:
  - if text index marker says searchable and vector store meta has valid `last_full_index_at`,
    mark `Full`;
  - else if text index marker exists but vector store is missing, mark `Partial`;
  - else default to `Building`.
- Before a `vektor index` re-run, set that project to `Building`, then advance `Partial` and
  `Full` at each successful pass.
- Keep lock scope tight: load status/phase, drop lock, then run async work.

## Acceptance criteria

- [ ] `IndexPhase` has exactly `Building`, `Partial`, `Full`, mapping to
  `"building"`, `"partial"`, `"full"`.
- [ ] Tracker state is per-project: A change to project B does not change project A status.
- [ ] On startup, tracker initializes from on-disk state:
  `Full` if both tiers are ready, `Partial` if shallow-only is ready, otherwise `Building`.
- [ ] A fresh per-project entry reads `Building`; `mark_partial()` then reads `Partial`; `mark_full()`
  then reads `Full`.
- [ ] Transitions are forward-only within a pass (`Building→Partial→Full`), with optional explicit
  reset to `Building` on re-index start.
- [ ] With project status `Building`, search returns empty results + index-building message and does
  not load/bind the embedder.
- [ ] With project status `Partial`, search returns keyword/shallow results and tags
  `index_status: "partial"`; with `Full`, returns hybrid and tags `"full"`.
- [ ] Shallow and deep indexers for the same project drive the shared tracker through
  `Building → Partial → Full`.
- [ ] Search and context handlers for multiple projects each reflect their own project status.
- [ ] No `RwLock` guard is held across an `.await`.
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test index_status
cargo test mcp::handlers                     # status strings preserved across all handlers
cargo test mcp::handlers::tests::keyword_search_does_not_build_embedder
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- The index marker files already exist (`vektor_text_index_ready` / `vektor_text_index_searchable`) and
  vector-store metadata (`vector_meta.json`) and can serve for startup health seeding; treat marker
  checks as non-invasive and test-backed.
- Keep the same string contract used by existing tests (`building`, `partial`, `full`) to avoid
  response-shape churn.
- `index_status` also appears in `search_code` responses as in Phase 4; route all MCP call sites
  through this per-project tracker.
