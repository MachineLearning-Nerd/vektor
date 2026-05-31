# Task 4.2 — `TextIndex::add_chunks(chunks)` (batch insert + commit)

**Phase**: 4 — Search + MCP
**Task ID**: 4.2
**PRD reference**: Section 4.10 (Tantivy), Section 4.3 ("batch Tantivy commit, once per window")
**Roadmap stage**: Stage 2 (interim — no release tag)
**Effort estimate**: S
**Depends on**: 4.1
**Blocks**: 4.3, 4.6

## Objective

Write chunks into the Tantivy index built by 4.1. Each chunk becomes one Tantivy
document keyed by its content-addressed `chunk_id`. Commits are batched (not
per-document) per PRD §4.3 so the orchestrator (4.6) controls when the index
becomes searchable.

## Inputs (must exist before starting)

- `TextIndex` + its schema/field handles from 4.1.
- The `Chunk` type from Phase 2 (`src/chunker/mod.rs`): `content`, `start_line`,
  `end_line`, `symbol_name: Option<String>`, `language: Option<Language>`, and the
  content-addressed chunk id used as LanceDB `id` (task 2.7). Map these into
  Tantivy fields.
- `delete_by_file`-style semantics: re-indexing a file deletes its old Tantivy
  docs before inserting (mirrors `VectorStore::delete_by_file`); provide a Tantivy
  `delete_by_file(rel_path)` here or in 4.6 so the orchestrator can keep a clean
  slate (PRD §4.5 step 2 / §4.3 "delete_by_file on Tantivy").

## Outputs (must exist after completion)

- `TextIndex::add_chunks(&mut self, chunks: &[ChunkDoc]) -> Result<()>` (or
  equivalent input type carrying `chunk_id`, `rel_path`, `content`,
  `symbol_name`, `language`, `start_line`, `end_line`, `index_depth`). Adds docs
  to the writer **without** committing.
- `TextIndex::commit(&mut self) -> Result<()>` — flushes the batched writer once.
- `TextIndex::delete_by_file(&mut self, rel_path: &str) -> Result<()>` —
  `delete_term` on the `rel_path` STRING field (clean slate before re-insert).
- `index_depth` always written as `"deep"` in Phase 4 (PRD §4.6 / phase README).

## Approach

- Acquire a single `IndexWriter` with a sane heap budget (e.g. 50 MB) for the
  batch; reuse it across `add_chunks` calls within one orchestration pass.
- For each chunk, build a `tantivy::Document` (`doc!` macro) setting every schema
  field. `symbol_name` is optional → write an empty string (or skip) when absent;
  `language` → its canonical lowercase string. `start_line`/`end_line` are `u64`.
- `delete_by_file` uses `writer.delete_term(Term::from_field_text(rel_path_field, rel_path))`.
- `commit` calls `writer.commit()` then reloads the reader so subsequent searches
  see the new docs. Do not commit per chunk or per file — commit once per pass.

## Acceptance criteria

- [ ] `add_chunks` adds N documents for N chunks; the docs are **not** searchable
      until `commit` is called (verifiable: search before commit → 0 hits; after
      commit → hits).
- [ ] `commit` makes all batched docs searchable in one flush; the reader reflects
      the new generation.
- [ ] `delete_by_file(rel_path)` followed by `commit` removes exactly that file's
      docs and leaves other files' docs intact.
- [ ] Re-indexing the same file (delete_by_file → add_chunks → commit) yields no
      duplicate `chunk_id` documents.
- [ ] `index_depth == "deep"` on every doc written in Phase 4.
- [ ] No `unwrap()` outside `#[cfg(test)]`; Tantivy errors carry context.

## Verification

```bash
cargo build
cargo test text_index::tests::add_chunks_then_commit_makes_searchable
cargo test text_index::tests::commit_is_batched_not_per_doc
cargo test text_index::tests::delete_by_file_is_scoped
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- Tantivy `IndexWriter` is not `Clone` and holds a memory budget; decide whether
  `TextIndex` owns a long-lived writer or creates one per batch. For Phase 4
  (one-shot index), per-orchestration-pass is simplest and matches PRD §4.3's
  "commit once per window."
- The `ChunkDoc` input shape should be the minimal projection 4.6 can build from
  a `Chunk` + its computed `chunk_id`; avoid coupling `TextIndex` to the full
  chunker types if a small input struct is cleaner.
