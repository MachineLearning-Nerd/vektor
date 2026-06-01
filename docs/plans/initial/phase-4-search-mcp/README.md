# Phase 4 — Search + MCP (interim — no release tag)

> **Goal**: Tantivy BM25 indexing + RRF hybrid fusion with adaptive weights + real MCP tool handlers. `search_code` now returns ranked results. `get_context_for_prompt` returns a basic context package (full assembly pipeline lives in Phase 5).

**Roadmap mapping**: Stage 2 — bridges `v0.3.0` (storage) to `v0.4.0` (full context assembly). **No release tag from this phase alone**; releases happen at the v0.4.0 boundary after Phase 5 lands.
**PRD mapping**: Section 4.2 (Hybrid Search + RRF), Section 4.6 (Two-Tier), Section 12 Week 4 Functions 4.1–4.8, Functions CA.10 + CA.11 (synonym, adaptive weights)
**Effort estimate**: 2–3 weeks of focused part-time work
**Status**: ⬜ Not started — per-task files written (expanded by task 3.12). Implementation starts from the linked task files, not from this summary table.

---

## Task list

| ID | Task | Effort | Depends on | Status |
|---|---|---|---|---|
| 4.1 | [`TextIndex::new(project_dir)` — Tantivy schema per PRD §4.10 (chunk_id / rel_path / content / symbol_name 2.0× boost / language / start_line / end_line / index_depth)](01-text-index-new.md) | M | 3.12 | ⬜ |
| 4.2 | [`TextIndex::add_chunks(chunks)` — batch insert + commit](02-text-index-add-chunks.md) | S | 4.1 | ⬜ |
| 4.3 | [`TextIndex::search(query, top_k)` — BM25 with en_stem tokenizer](03-text-index-search.md) | M | 4.2 | ⬜ |
| 4.4 | [`rrf_fuse(semantic, keyword, k)` + `AdaptiveWeights::compute` + `SynonymExpander` (~50 entries)](04-rrf-fuse-adaptive-weights-synonyms.md) | M | 3.12 | ⬜ |
| 4.5 | [`search_hybrid(query, config)` — tokio::join! of vector + text search → RRF fusion](05-search-hybrid.md) | M | 3.8, 4.3, 4.4 | ⬜ |
| 4.6 | [`index_codebase` orchestrator — EXTEND the Phase 3 vector-only handler to also write Tantivy BM25 (does not rebuild it)](06-index-codebase-orchestrator.md) | L | 3.7c, 4.2 | ⬜ |
| 4.7 | [MCP server: replace no-op handlers from task 1.6 with real `search_code` + `index_codebase` dispatch](07-mcp-server-bootstrap.md) | M | 1.6, 4.5, 4.6 | ⬜ |
| 4.8 | [Tool handlers: real `search_code` + basic `get_context_for_prompt` (search → top-k → return without dedup/budget; full assembly is Phase 5)](08-tool-handlers.md) | M | 4.5, 4.7 | ⬜ |

---

## Phase exit criteria

All must be true before moving to Phase 5:

- [ ] All 8 tasks above marked ✅ Done
- [ ] `vektor index <repo>` builds both LanceDB AND Tantivy indices in one pass
- [ ] `vektor serve` + MCP `search_code` call returns real ranked results (not no-op JSON)
- [ ] Hybrid mode produces different rankings than semantic-only or keyword-only on a known query (verifiable test)
- [ ] Adaptive weights kick in: identifier-heavy queries (`validate_token AuthMiddleware`) favor BM25; natural-language queries (`how does authentication work`) favor semantic
- [ ] Synonym expansion: querying `"auth"` finds chunks containing `"authentication"` via BM25 expansion
- [ ] `get_context_for_prompt` returns a structured `ContextPackage` JSON matching PRD §5.3 (even if budget allocation is naive at this stage)
- [ ] No regression on Phase 2/3 tests
- [ ] Search latency <300ms P95 on a 10K-chunk index (task 4.5 owns the deterministic perf smoke; no IVF_PQ yet — brute force is fine at this scale)

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
