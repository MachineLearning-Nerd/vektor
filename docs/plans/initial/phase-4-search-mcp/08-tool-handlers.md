# Task 4.8 — Tool handlers: `search_code` + basic `get_context_for_prompt`

**Phase**: 4 — Search + MCP
**Task ID**: 4.8
**PRD reference**: Section 5.3 (ContextPackage), Section 8 (search_code modes), Section 12 Week 4 Function 4.8
**Roadmap stage**: Stage 2 (interim — no release tag)
**Effort estimate**: M
**Depends on**: 4.5, 4.7
**Blocks**: 5.6

## Objective

Shape the real MCP responses for the two remaining stubbed tools. `search_code`
returns ranked hybrid results as PRD-shaped JSON. `get_context_for_prompt`
returns a **deliberately naive** `ContextPackage` (search → top-k → return, with
NO dedup, expansion, or budget allocation — those are Phase 5). The point is that
neither tool is a no-op after this task.

> Note: the phase README lists 4.8's `Depends on` as `4.5, 4.7`; DEPENDENCIES.md
> lists `4.5, 4.7, 5.5`. The `5.5` edge is the *full* assembly pipeline, which is
> explicitly out of scope here — the Phase 4 `get_context_for_prompt` is naive by
> design. Treat the executable dependency as `4.5, 4.7`; the 5.5 edge is the
> Phase 5 upgrade of this same handler (task 5.6). This file follows the phase
> README (4.5, 4.7) and flags the drift.

## Inputs (must exist before starting)

- `search_hybrid` (4.5) + the server state plumbing from 4.7.
- The `search_code` modes (`hybrid` | `semantic` | `keyword`) and parameters
  (query, top_k, optional filters) from PRD §8.
- The `ContextPackage` JSON shape from PRD §5.3 (results with `relevance_score`,
  plus `index_status` / `index_coverage_pct` per PRD §4.4/§5.3).

## Outputs (must exist after completion)

- `handle_search_code(args)` parses `query`, optional `mode` (default `hybrid`),
  optional `top_k`, runs `search_hybrid`, returns JSON: an array of ranked results
  (`chunk_id`, `rel_path`, line range, `symbol_name`, `language`, `content`,
  `relevance_score`) plus `index_status`. Errors → JSON error object (mirror
  `handle_index_codebase`'s pattern), never panic, never stdout.
- `handle_get_context_for_prompt(args)` parses the prompt, runs `search_hybrid`,
  returns a `ContextPackage`-shaped JSON: top-k results assembled into the package
  fields PRD §5.3 defines, WITHOUT dedup/expansion/budget. Include
  `index_status` and a placeholder `result_confidence` (naive at Phase 4).

## Approach

- Mirror the existing `run_index_tool` arg-parsing/error-mapping pattern in
  `src/mcp/handlers.rs` (missing/typed args → JSON error string; success → shaped
  JSON). Keep handlers thin: parse → call engine → shape JSON.
- For `search_code`, map `HybridResult` → the PRD §8 result object; pass `mode`
  through to `search_hybrid`.
- For `get_context_for_prompt`, build the minimal `ContextPackage`: take top-k
  hybrid results, emit them as the package's result list with `relevance_score`,
  set `index_status` from whether a full index exists, and a naive
  `result_confidence` (e.g. High if top score above a threshold). Explicitly do
  NOT implement TokenCounter/Deduplicator/RelatedExpander/budget — those are
  Phase 5 (tasks 5.1–5.5) and upgrade this handler at task 5.6.
- Keep both handlers stdout-silent; log via tracing.

## Acceptance criteria

- [ ] `search_code` returns real ranked results (not the "not implemented" stub)
      and honors `mode` (hybrid/semantic/keyword).
- [ ] `get_context_for_prompt` returns a structured `ContextPackage` JSON matching
      PRD §5.3 (even with naive budget/confidence) — not a stub.
- [ ] Both handlers return a JSON error object on bad/missing args; neither panics
      nor writes to stdout.
- [ ] `index_status` present in both responses (PRD §4.4/§5.3).
- [ ] No dedup/expansion/budget logic is added here (kept for Phase 5).
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test mcp::handlers::tests::search_code_returns_ranked_results
cargo test mcp::handlers::tests::get_context_returns_context_package
cargo test mcp::handlers::tests::handlers_reject_bad_args_as_json_error
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- `get_context_for_prompt` is intentionally naive at Phase 4 (phase README): the
  handler exists so the tool isn't a no-op; the QUALITY (dedup, expansion,
  token-budgeted assembly) lands in Phase 5. Do not gold-plate it here.
- Use the fake-embedder test seam (as Phase 3's `index_codebase` test does) so
  handler tests run without a downloaded model.
- This task closes Phase 4: on completion, run the AGENTS.md phase-completion
  checklist (mark all 8 tasks ✅ with hashes; update README/DEPENDENCIES/ROADMAP
  current-state; expand `phase-5-context-assembly/` per-task files). No release
  tag — the next tag is `v0.4.0` after Phase 5.
