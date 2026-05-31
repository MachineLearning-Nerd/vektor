# Task 4.4 — `rrf_fuse` + `AdaptiveWeights::compute` + `SynonymExpander`

**Phase**: 4 — Search + MCP
**Task ID**: 4.4
**PRD reference**: Section 4.2 (RRF fusion k=60, adaptive weights v2.2.1, static synonym expansion v2.2)
**Roadmap stage**: Stage 2 (interim — no release tag)
**Effort estimate**: M
**Depends on**: 3.12
**Blocks**: 4.5

## Objective

Implement the three pure-logic pieces that turn two ranked result lists (semantic
+ keyword) into one fused ranking: Reciprocal Rank Fusion with adaptive
semantic/keyword weighting based on query type, plus a small static synonym map
that expands the BM25 query for recall. These are CPU-only, no I/O — the easiest
Phase 4 tasks to unit-test in isolation, which is why they are unblocked
independently of 4.1–4.3.

## Inputs (must exist before starting)

- The RRF formula and constant from PRD §4.2: `rrf_score(d) = Σ 1/(k + rank_i(d))`,
  `k = 60`.
- The adaptive-weight density rules from PRD §4.2 (v2.2.1, see Approach).
- The ~50-entry static synonym map shape from PRD §4.2 (`auth`, `db`, `api`,
  `config`, ...). PRD §5.10/5.11 (DEPENDENCIES) note the SynonymExpander and
  AdaptiveWeights are owned here, not as separate later tasks.

## Outputs (must exist after completion)

- `rrf_fuse(semantic: &[RankedId], keyword: &[RankedId], k: u32, weights: AdaptiveWeights) -> Vec<FusedHit>`
  — combines by `chunk_id`, summing weighted reciprocal-rank contributions; a doc
  present in only one list still scores.
- `AdaptiveWeights { semantic: f32, keyword: f32 }` + `AdaptiveWeights::compute(query) -> AdaptiveWeights`
  using identifier-density classification (below).
- `SynonymExpander` with a curated static map (~50 entries) and
  `expand(query) -> String` (or token list) producing the OR-expanded BM25 query.
  Semantic search uses the **original** query, not the expanded one.
- `RRF_K` constant = 60.

## Approach

- **RRF**: build a `HashMap<chunk_id, f32>`. For each list, for each item at
  0-based `rank` (so position 0 → rank 1), add `weight * 1.0 / (k + rank + 1)`.
  Sum across both lists; sort descending. Ties broken deterministically (e.g. by
  `chunk_id`).
- **AdaptiveWeights::compute** (PRD §4.2 v2.2.1 density classification — NOT binary):
  - Count identifier tokens (match `[a-z]+_[a-z]+` snake_case, `[a-z]+[A-Z]`
    camelCase, or `\w+\.\w+` dotted) among total tokens.
  - density = identifier_tokens / total_tokens.
  - density > 0.60 → `semantic=0.4, keyword=0.6` (e.g. "validate_token AuthMiddleware")
  - density < 0.25 → `semantic=0.7, keyword=0.3` (e.g. "how does authentication work")
  - 0.25–0.60 (mixed) → `semantic=0.6, keyword=0.4` (e.g. "how does validate_token handle expired JWTs")
  - Empty query → fall back to the mixed default.
- **SynonymExpander**: a `const`/`static` curated map (~50 entries; do not pad).
  `expand` lowercases + tokenizes the query, replaces each known token with an
  OR-group of its synonyms, and joins into a Tantivy-parseable OR query. Unknown
  tokens pass through unchanged. ~3–8ms cost target per PRD §4.2.

## Acceptance criteria

- [ ] `rrf_fuse` reproduces the PRD formula: a doc ranked #1 in both lists scores
      `w_sem/(60+1) + w_kw/(60+1)`; a doc in only one list still appears.
- [ ] `AdaptiveWeights::compute` returns the three documented weight pairs for the
      three documented example queries; weights sum to 1.0.
- [ ] A single identifier inside a natural-language question does NOT flip weights
      to keyword-heavy (density stays < 0.60) — the v2.2.1 anti-flip property.
- [ ] `SynonymExpander::expand("auth")` produces an OR query including
      `authentication`, `login`, `token`, etc.; an unknown token is unchanged.
- [ ] Map has ~50 curated entries, code-concept focused (no filler).
- [ ] All three are pure functions (no I/O); fully unit-tested.

## Verification

```bash
cargo build
cargo test search::rrf::tests
cargo test search::weights::tests
cargo test search::synonyms::tests
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- RRF consumes **rank**, not raw scores — do not normalize BM25 vs cosine scores.
- The synonym map is applied to the BM25 query ONLY (PRD §4.2). The semantic
  query is the user's original text so the embedder isn't fed expansion noise.
- `AdaptiveWeights` and `SynonymExpander` are the same components DEPENDENCIES.md
  lists as 5.11 / 5.10 "(in 4.4)" — they ship here, Phase 5 just consumes them.
