# Task 4.3 — `TextIndex::search(query, top_k)` (BM25)

**Phase**: 4 — Search + MCP
**Task ID**: 4.3
**PRD reference**: Section 4.10 (BM25 field boosting), Section 4.2 (keyword half of hybrid)
**Roadmap stage**: Stage 2 (interim — no release tag)
**Effort estimate**: M
**Depends on**: 4.2
**Blocks**: 4.5

## Objective

Run BM25 keyword search over the Tantivy index, with a 2.0× boost on
`symbol_name`, returning ranked results that 4.5 fuses with the semantic results
from `VectorStore::search`.

## Inputs (must exist before starting)

- A committed `TextIndex` with documents (4.1 + 4.2).
- `en_stem` tokenizer registered (4.1), so query terms stem-match indexed terms.
- The `symbol_name` 2.0× boost requirement from PRD §4.10.

## Outputs (must exist after completion)

- `TextIndex::search(&self, query: &str, top_k: usize) -> Result<Vec<KeywordHit>>`
  where `KeywordHit` carries at least `chunk_id`, `rel_path`, `score` (BM25), and
  the stored display fields (`start_line`, `end_line`, `symbol_name`, `language`).
- Ranking reflects BM25 with `symbol_name` weighted 2.0× over `content`.
- `top_k == 0` short-circuits to an empty vec; empty/whitespace query returns an
  empty vec (no panic).

## Approach

- Build a `QueryParser` over the `content` and `symbol_name` fields and set the
  field boost for `symbol_name` to `2.0` (`QueryParser::set_field_boost`), or use
  a `BoostQuery` wrapping the `symbol_name` sub-query. Either path must produce
  the 2.0× weighting from PRD §4.10.
- Use the reader's `searcher.search(&query, &TopDocs::with_limit(top_k))`.
- For each `(score, doc_address)`, retrieve the stored doc and project the fields
  into `KeywordHit`. The `score` is the raw BM25 score; 4.4 converts ranks (not
  raw scores) into RRF contributions, so passing the rank order through is what
  matters — but keep the score for debugging/telemetry.
- Tokenize the query with the same `en_stem` tokenizer (the `QueryParser` does
  this when the field uses that tokenizer). Synonym expansion is **not** done here
  — that is 4.4's `SynonymExpander`, applied to the query string before it reaches
  this function (BM25-only; semantic search uses the original query).

## Acceptance criteria

- [ ] A query matching a `symbol_name` ranks the symbol-name hit above an
      otherwise-equal `content`-only hit (verifies the 2.0× boost).
- [ ] BM25 ordering is stable and deterministic for a fixed corpus + query.
- [ ] `top_k` caps the result count; `top_k == 0` and empty queries return `[]`.
- [ ] `KeywordHit.chunk_id` equals the LanceDB `id` for the same chunk (cross-store
      join works) — verified by indexing the same chunk into both stores in a test.
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test text_index::tests::search_boosts_symbol_name
cargo test text_index::tests::search_respects_top_k_and_empty
cargo test text_index::tests::search_is_deterministic
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- Tantivy 0.26 BM25 is the default similarity; no extra config needed beyond the
  field boost. Verify the `set_field_boost` vs `BoostQuery` API against
  `tantivy 0.26` docs before writing (the API has shifted across 0.2x releases).
- RRF (4.4) consumes **rank position**, not raw BM25 score, so normalization
  across the two retrievers is unnecessary — this is the whole point of RRF.
