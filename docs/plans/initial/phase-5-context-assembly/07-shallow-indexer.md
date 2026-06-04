# Task 5.7 — `ShallowIndexer::build` — fast first-result BM25 tier

**Phase**: 5 — Context Assembly
**Task ID**: 5.7
**PRD reference**: Section 4.4 (Two-Tier Indexing Strategy)
**Roadmap stage**: Stage 2 / v0.4.0
**Effort estimate**: M
**Depends on**: 2.6, 4.1
**Blocks**: 5.8

## Objective

Make the first BM25 results available on a brand-new project in **under 5s**,
before the deep (semantic) index finishes. `ShallowIndexer::build(path)` walks
the directory tree and writes a Tantivy index built from a cheap, content-light
projection of each file — the path itself, the first 50 lines, the last 20
lines, and regex-extracted declarations — so `search_code` (keyword mode) and
`get_context_for_prompt` (partial results) can answer immediately while
tree-sitter chunking + embedding run later. This is the "Phase 1: Shallow Index
(0-5s)" tier of PRD §4.4; the deep pass (4.6) later overwrites these docs.

## Inputs (must exist before starting)

- `TextIndex` (4.1) with project-scoped open/create, `add_chunks`, `commit`,
  and the schema field `index_depth` (`src/text_index.rs` —
  `TextIndexFields::index_depth`; the deep pass writes `index_depth = "deep"`).
- `discover_files(root, config)` (2.1, `src/discovery.rs`) — the same ignore-aware
  walk the deep pass uses, so shallow and deep see the same file set.
- Sliding-window concepts (2.6, `src/chunker/sliding.rs`) for the line-window
  framing — shallow does NOT run tree-sitter; it lifts only the head/tail line
  windows and declaration lines.
- `Chunk` (`src/chunker::Chunk`) — `TextIndex::add_chunks` stores `Chunk::id`,
  `rel_path`, line range, optional `symbol_name`, and language.

## Outputs (must exist after completion)

- `ShallowIndexer::build(path: &Path, config: &Config) -> Result<ShallowStats>`
  (or method on a `ShallowIndexer` struct) that:
  - walks `path` via `discover_files`,
  - for each file emits one (or few) shallow Tantivy doc(s) whose searchable
    content is `first 50 lines + last 20 lines + regex-extracted declarations`,
    plus the relative path as a searchable field,
  - tags every shallow doc with `index_depth = "shallow"` (new constant
    alongside the existing `"deep"` value in `src/text_index.rs`),
  - commits the Tantivy index once at the end of the walk (batch commit, per the
    4.6 "commit once per pass" rule), then marks it searchable.
- A declaration extractor: regex (or equivalent line scan) that pulls
  language-agnostic declaration lines — e.g. `fn`/`def`/`class`/`struct`/`func`/
  `impl`/`interface`/`type`/`export` heads — from each file so symbol names are
  keyword-searchable before AST chunking exists.
- `ShallowStats` (or additive fields) reporting files walked + shallow docs
  written, so the caller can log shallow-tier completion.
- A stable, deterministic shallow `chunk_id` per file so the deep pass's
  `delete_by_file` (4.2) cleanly removes the shallow docs for a file before it
  inserts that file's deep chunks — shallow and deep never coexist for one file.

## Approach

- Reuse `discover_files` exactly — do not re-implement the walk or the ignore
  rules. Shallow and deep must agree on which files exist.
- For each file: read it once, take `lines[..50]` and `lines[len-20..]` (clamped
  for short files; do not double-count when a file is <70 lines), scan lines for
  declaration patterns, and concatenate into one shallow content blob. Frame it
  as a `Chunk` (start/end line = the windowed range) so `TextIndex::add_chunks`
  can store it without a second schema. Keep it content-light — the point is
  speed, not recall depth.
- Build declarations with a small static set of per-language regexes keyed off
  the file extension (same extension→language mapping the chunker already uses);
  fall back to a generic "leading-keyword" line match for unknown languages.
- Write all docs to the writer, then a single `commit()` after the loop — never
  per file (mirrors 4.6 §4.3 batch-commit semantics).
- Set `index_depth = "shallow"` on every shallow doc. Phase exit relies on the
  deep pass (4.6) overwriting these with `"deep"` via delete-then-insert; this
  task only writes the shallow tier and leaves a clean handoff.
- Skip files that fail to read (mark/log, continue) — one unreadable file must
  not abort the <5s shallow pass.

## Acceptance criteria

- [ ] `ShallowIndexer::build` walks a fixture project via `discover_files` and
      writes one shallow Tantivy doc per discovered file.
- [ ] Shallow content includes the path, first 50 lines, last 20 lines, and at
      least the regex-extracted declaration lines of each file.
- [ ] Files shorter than 70 lines do not duplicate overlapping head/tail lines.
- [ ] Every shallow doc carries `index_depth = "shallow"`; no shallow doc carries
      `"deep"`.
- [ ] After `build`, the Tantivy index is committed once and a keyword query for
      a known declaration name returns the matching file (BM25 results available).
- [ ] On a representative fixture the shallow pass completes well under the 5s
      target (assert via a generous time bound in the test, not a flaky tight one).
- [ ] A subsequent deep `delete_by_file(rel_path)` removes that file's shallow
      docs so shallow and deep tiers never coexist for one file.
- [ ] An unreadable file is skipped without aborting the pass.
- [ ] No `unwrap()` outside `#[cfg(test)]`.

## Verification

```bash
cargo build
cargo test shallow                       # ShallowIndexer build + declaration extraction
cargo test text_index                    # TextIndex add/commit still green (no schema regression)
cargo test discovery                     # shared walk unchanged
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- **Shallow is keyword-only by design** (PRD §4.4): no embeddings, no LanceDB
  writes here. `search_code` semantic/hybrid modes stay unavailable until the
  deep tier lands — that gating is surfaced by `index_status` (task 5.8), not by
  this indexer.
- The `index_depth` field already exists in the 4.1 schema with a `"deep"`
  constant; this task adds the `"shallow"` value. Confirm whether existing
  searches must filter by `index_depth` or whether shallow/deep docs for the
  same chunk are guaranteed mutually exclusive by delete-then-insert (the
  intended invariant — flag if a search-time filter is needed instead).
- Open question: should shallow run synchronously before `vektor serve` accepts
  queries, or as the first background step? PRD §4.4 says "search available
  immediately (keyword mode)"; the orchestration of shallow→deep and the
  status transitions are task 5.8's concern — keep `build` a callable unit here.
