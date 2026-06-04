# Phase 4 — Search + MCP (interim — no release tag)

> **Goal**: Tantivy BM25 indexing + RRF hybrid fusion with adaptive weights + real MCP tool handlers. `search_code` now returns ranked results. `get_context_for_prompt` returns a basic context package (full assembly pipeline lives in Phase 5).

**Roadmap mapping**: Stage 2 — bridges `v0.3.0` (storage) to `v0.4.0` (full context assembly). **No release tag from this phase alone**; releases happen at the v0.4.0 boundary after Phase 5 lands.
**PRD mapping**: Sections 4.2, 4.3, 4.5, 4.10, 9, and 12 Week 4 Functions 4.1–4.8, plus CA.10 + CA.11 (synonyms and adaptive weights)
**Effort estimate**: 2–3 weeks of focused part-time work
**Status**: ✅ **Done** — implemented + merged to main (PR #4 `1b39f49`; review-fix PRs #5/#6 `7cfd8e9` / `c0560e3`). All 8 tasks are backed by tests; the modules live in `src/text_index.rs`, `src/search/*`, and `src/mcp/handlers.rs`.

---

## Task list

| ID | Task | Effort | Depends on | Status |
|---|---|---|---|---|
| 4.1 | [`TextIndex::new(project_dir)` — Tantivy schema per PRD §4.10 (chunk_id / rel_path / content / symbol_name 2.0× boost / language / start_line / end_line / index_depth)](01-text-index-new.md) | M | 3.12 | ✅ `1b39f49` |
| 4.2 | [`TextIndex::add_chunks(chunks)` — batch insert + commit](02-text-index-add-chunks.md) | S | 4.1 | ✅ `1b39f49` |
| 4.3 | [`TextIndex::search(query, top_k)` — BM25 with en_stem tokenizer](03-text-index-search.md) | M | 4.2 | ✅ `1b39f49` |
| 4.4 | [`rrf_fuse(semantic, keyword, k)` + `AdaptiveWeights::compute` + `SynonymExpander` (~50 entries)](04-rrf-fuse-adaptive-weights-synonyms.md) | M | 3.12 | ✅ `1b39f49` |
| 4.5 | [`search_hybrid(query, config)` — tokio::join! of vector + text search → RRF fusion](05-search-hybrid.md) | M | 3.8, 4.3, 4.4 | ✅ `1b39f49` |
| 4.6 | [`index_codebase` orchestrator — EXTEND the Phase 3 vector-only handler to also write Tantivy BM25 (does not rebuild it)](06-index-codebase-orchestrator.md) | L | 3.7c, 4.2 | ✅ `1b39f49` |
| 4.7 | [MCP server: replace no-op handlers from task 1.6 with real `search_code` + `index_codebase` dispatch](07-mcp-server-bootstrap.md) | M | 1.6, 4.5, 4.6 | ✅ `1b39f49` |
| 4.8 | [Tool handlers: real `search_code` + basic `get_context_for_prompt` (search → top-k → return without dedup/budget; full assembly is Phase 5)](08-tool-handlers.md) | M | 4.5, 4.7 | ✅ `1b39f49` |

---

## Execution order

1. Start the two independent Phase 4 lanes after 3.12:
   - Text index lane: 4.1 -> 4.2 -> 4.3.
   - Ranking helper lane: 4.4.
2. Join both lanes in 4.5, where vector search, BM25 search, adaptive weights, synonyms, and RRF become one hybrid search path.
3. Extend indexing in 4.6 after 4.2 so the existing Phase 3 vector-only `index_codebase` flow writes LanceDB and Tantivy in one pass.
4. Replace MCP no-op behavior in 4.7 after both hybrid search and dual-store indexing exist.
5. Close Phase 4 with 4.8 by wiring real `search_code` and a basic `get_context_for_prompt` response. Deduplication, expansion, token-budget allocation, and full context quality remain Phase 5 work.

---

## PRD/Roadmap coverage map

| Source requirement | Covered by | Notes |
|---|---|---|
| PRD §4.2 Hybrid Search + RRF Fusion | 4.3, 4.4, 4.5 | Covers BM25 search, RRF `k=60`, adaptive query weighting, and static synonym expansion. |
| PRD §4.3 Data Flow + §4.5 Delete-Then-Insert | 4.2, 4.6 | Keeps LanceDB and Tantivy writes in the same indexing pass, with batched Tantivy commit semantics and stale-entry replacement. |
| PRD §4.10 storage schema definitions | 4.1, 4.2, 4.3, 4.6 | Adds the Tantivy schema and document writes for `chunk_id`, `rel_path`, `content`, boosted `symbol_name`, `language`, line range, and `index_depth`. |
| PRD §9 MCP Tools API | 4.6, 4.7, 4.8 | Implements the Stage 2 primary tools: `index_codebase`, `search_code`, and `get_context_for_prompt`. |
| PRD §12 Week 4 Functions 4.1-4.8 | 4.1-4.8 | One task file exists for each Week 4 function, with dependencies mirrored in `DEPENDENCIES.md`. |
| Roadmap Stage 2 core engine (`v0.4.x`) | 4.1-4.8, then Phase 5 | Phase 4 delivers BM25, hybrid search, and real MCP handlers; Phase 5 completes token-budgeted context assembly before the `v0.4.0` release boundary. |
| Roadmap Stage 2 launch prerequisites | Phase 6 tasks 6.1-6.5 | `vektor init`, signed binaries, release checksums, benchmark gate, and `BENCHMARKS.md` are intentionally outside Phase 4 and remain tracked in Phase 6. |
| PRD two-tier shallow/deep indexing | 4.1, 4.6, Phase 5 task 5.7 | Phase 4 writes `index_depth = "deep"` and keeps schema compatibility; ShallowIndexer behavior lands in Phase 5. |

---

## Phase exit criteria

All must be true before moving to Phase 5:

- [x] All 8 tasks above marked ✅ Done
- [x] `vektor index <repo>` builds both LanceDB AND Tantivy indices in one pass
- [x] `vektor serve` + MCP `search_code` call returns real ranked results (not no-op JSON)
- [x] Hybrid mode produces different rankings than semantic-only or keyword-only on a known query (verifiable test)
- [x] Adaptive weights kick in: identifier-heavy queries (`validate_token AuthMiddleware`) favor BM25; natural-language queries (`how does authentication work`) favor semantic
- [x] Synonym expansion: querying `"auth"` finds chunks containing `"authentication"` via BM25 expansion
- [x] `get_context_for_prompt` returns a structured `ContextPackage` JSON matching PRD §5.3 (even if budget allocation is naive at this stage)
- [x] No regression on Phase 2/3 tests
- [x] Search latency <300ms P95 on a 10K-chunk index (task 4.5 owns the deterministic perf smoke; no IVF_PQ yet — brute force is fine at this scale)

> ✅ Verified via the test suite (PRs #4/#5/#6, all merged). Model/scale-gated
> criteria (`vektor index` end-to-end on a real repo, real 10K-chunk latency) are
> covered by deterministic tests + the `ten_k_chunk_latency_p95_under_300ms` perf
> smoke and the fake-embedder orchestrator test — mirroring how Phase 3 handled
> model-dependent criteria. Full local gate (`scripts/ci.sh`) re-run on closure.

**No release tag from this phase.** The next release tag is `v0.4.0` after Phase 5 completes.

---

## Notes

- **rmcp 1.7 handler signatures**: task 1.6 already wired the no-op handlers. This phase replaces the no-op bodies with calls into `search_hybrid` and the naive Phase 4 context response. Tool names and input schemas stay stable; human-readable descriptions/server instructions should stop advertising no-op `v0.1.0` behavior once the handlers are real.
- **Tantivy commit semantics**: writes are batched. `add_chunks` doesn't commit immediately; commit only after the orchestrator processes all files (per PRD §4.3 "batch Tantivy commit (once per debounce window, not per file)"). At Phase 4, debounce isn't a thing yet, so commit at end of `index_codebase`.
- **RRF k constant**: 60 (PRD §4.2 standard).
- **Synonym map size**: ~50 entries per PRD §4.2 "Static synonym expansion." Don't pad it — small, curated, focused on code concepts.
- **Field boost on `symbol_name`**: 2.0x per PRD §4.10. Tantivy supports this via `BoostQuery` or schema-level field weights.
- **Stale `index_depth` field**: the Tantivy schema includes `index_depth` for shallow-vs-deep distinction. At Phase 4 we only have deep, so always write `"deep"`. ShallowIndexer arrives in Phase 5.
- **`get_context_for_prompt` is intentionally naive at Phase 4**: it returns search results without dedup, expansion, or budget allocation. Those are Phase 5 deliverables. The handler exists so that the MCP tool isn't a no-op, but the *quality* improvements land in Phase 5.

---

## When this phase completes

1. Mark all tasks ✅
2. (No tag — release happens after Phase 5)
3. Update Current State tables
4. Expand `phase-5-context-assembly/` from task list to per-task files (task 4.8 closes this)
