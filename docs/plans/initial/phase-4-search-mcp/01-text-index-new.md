# Task 4.1 — `TextIndex::new(project_dir)` (Tantivy schema)

**Phase**: 4 — Search + MCP
**Task ID**: 4.1
**PRD reference**: Section 4.10 (Full-Text Index — Tantivy schema v2.2.1)
**Roadmap stage**: Stage 2 (interim — no release tag)
**Effort estimate**: M
**Depends on**: 3.12
**Blocks**: 4.2

## Objective

Create the `TextIndex` type backing Tantivy BM25 search, with the schema from PRD
§4.10. This is the keyword half of hybrid search; `VectorStore` (Phase 3) is the
semantic half. `TextIndex::new` opens (or creates) a Tantivy index under the
per-project Vektor data directory and exposes the schema fields that later tasks
(`add_chunks`, `search`) write and query.

## Inputs (must exist before starting)

- `tantivy = "0.26"` (already a dependency in `Cargo.toml` — do not bump or add).
- The per-project data-dir convention used by `VectorStore::new`
  (`~/.vektor/<project>/...`); mirror its project-scoping so LanceDB and Tantivy
  live side by side (e.g. `.../tantivy/` next to `.../lance/`).
- `src/error.rs` `VektorError` taxonomy (use `Storage`/`State` variants for
  Tantivy open/commit failures; do not invent a new variant unless justified).

## Outputs (must exist after completion)

- A `TextIndex` type (suggested module `src/text_index/mod.rs` or `src/text_index.rs`)
  with `TextIndex::new(project_dir, config) -> Result<TextIndex>`.
- A schema builder exposing these fields (PRD §4.10):
  - `chunk_id` — Utf8, STORED, not indexed (cross-store join key to LanceDB `id`)
  - `rel_path` — Utf8, STORED + STRING (not tokenized; exact-match filtering)
  - `content` — Utf8, TEXT, tokenized with `en_stem`; BM25 full-text search
  - `symbol_name` — Utf8, TEXT (tokenized) + STORED; boosted 2.0× in BM25 scoring
  - `language` — Utf8, STRING, STORED (exact-match filtering)
  - `start_line` — u64, STORED
  - `end_line` — u64, STORED
  - `index_depth` — Utf8, STRING, STORED ("shallow" | "deep")
- Field handles (`tantivy::schema::Field`) retained on the struct (or
  re-derivable from the stored `Schema`) so 4.2/4.3 can reference them by name.

## Approach

- Build a `tantivy::schema::Schema` via `SchemaBuilder` with the field options
  above. Register the `en_stem` tokenizer for `content` (and `symbol_name`) via
  `TextFieldIndexing::set_tokenizer("en_stem")` + `IndexRecordOption` that
  retains term frequencies + positions (BM25 needs freqs).
- Open the index with `Index::open_or_create(MmapDirectory::open(path), schema)`;
  create the directory if missing. Register the default `en_stem` tokenizer in
  the index's `TokenizerManager` if not already present.
- Store the `Index`, `Schema`, and field handles on `TextIndex`. Defer creating a
  long-lived `IndexWriter` to 4.2 (writer holds a heap budget; create it where
  `add_chunks` needs it).
- Keep `index_depth` a first-class field even though Phase 4 only ever writes
  `"deep"` (ShallowIndexer is Phase 5) — the schema must be forward-compatible.

## Acceptance criteria

- [ ] `TextIndex::new` creates a Tantivy index dir under the project's Vektor
      data dir and is idempotent (second call opens the existing index, does not
      wipe it).
- [ ] The schema contains exactly the 8 fields above with the PRD-specified
      options (TEXT+en_stem for `content`/`symbol_name`; STRING for
      `rel_path`/`language`/`index_depth`; STORED u64 lines; STORED-only
      `chunk_id`).
- [ ] `content` and `symbol_name` use the `en_stem` tokenizer with term
      frequencies retained (verifiable by indexing one doc and confirming BM25
      returns a nonzero score in 4.3 — at 4.1, assert the tokenizer name on the
      field's `IndexingOptions`).
- [ ] No `unwrap()` outside `#[cfg(test)]`; Tantivy errors map to a `VektorError`
      variant with context.

## Verification

```bash
cargo build
cargo test text_index::tests::new_creates_index
cargo test text_index::tests::schema_has_expected_fields
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Notes / open questions

- The Tantivy schema is the storage contract for 4.2/4.3 (and 4.6 when the
  orchestrator writes both stores). If a field option changes here, update those
  tasks.
- `chunk_id` MUST equal the LanceDB `id` (content-addressed `sha256(...)` from
  Phase 2 task 2.7) so hybrid fusion (4.4/4.5) can join semantic + keyword hits
  by id. Do not generate a separate Tantivy-local id.
- Keep the data-dir layout consistent with `VectorStore` so a future `delete`/
  `wipe` can clear both stores by project.
