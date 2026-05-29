# Phase 3 — Embedding + Storage (`v0.3.0`)

> **Goal**: ONNX Jina v2 Base Code loads from `~/.vektor/models/`, embeds chunks via mean-pooling + L2 normalization, stores them in an embedded LanceDB table with proper Arrow schema. OpenAI-compatible cloud backend works as alternate. Secret-aware indexing skips obvious secrets before embedding.

**Roadmap mapping**: Stage 2 / `v0.3.0`
**PRD mapping**: Section 4.2 (Embedder), Section 4.10 (LanceDB schema), Section 6 (Embedding backends), Section 12 Week 3 Functions 3.1–3.9, plus brought-forward B1.2/B1.5 (SecretDetector) and B6.1 (`vektor models download`)
**Effort estimate**: 4–6 weeks of focused part-time work — this is the hardest phase due to ONNX integration
**Status**: ⬜ Not started — readiness only. Per-task files are NOT yet written; expand them only after the Phase 2 publish gate is explicitly approved.

---

## Task list

Do not start implementation from this table directly. These rows are the phase-level roadmap; task files must be written and reviewed before any Phase 3 code changes.

| ID | Task | Effort | Depends on | Status |
|---|---|---|---|---|
| 3.1 | `Embedder` trait — async, `embed/dim/name/prefix_for_document/prefix_for_query` | S | 2.9 | ⬜ |
| 3.2 | `OnnxEmbedder::new(model_name)` — load ONNX model + `tokenizer.json` via `ort` + `tokenizers`; warm-up | L | 3.1 | ⬜ |
| 3.3 | `OnnxEmbedder::embed(texts)` — prefix prepend → tokenize → run session → mean-pool → L2-normalize → batch in 32 | L | 3.2 | ⬜ |
| 3.4 | `OpenAiCompatEmbedder::embed` — HTTP POST to `/v1/embeddings`, exponential backoff on 429 | M | 3.1 | ⬜ |
| 3.5 | `build_embedder(config) -> Box<dyn Embedder>` factory with fallback-to-ONNX behavior | S | 3.2, 3.3, 3.4 | ⬜ |
| 3.6 | `VectorStore::new(project_dir, dim)` — LanceDB connect, create table with Arrow schema from PRD Section 4.10 | M | 2.9 | ⬜ |
| 3.7 | `VectorStore::reindex_file(rel_path, chunks, embedder)` — delete-then-insert with chunk-level embedding cache | XL → split into 3.7a (read-before-delete), 3.7b (delete_by_file + reuse map), 3.7c (insert RecordBatch) | 3.5, 3.6 | ⬜ |
| 3.8 | `VectorStore::search(query_vec, top_k, filter)` — ANN query via LanceDB `nearest_to`, SQL-like filter | M | 3.6 | ⬜ |
| 3.9 | `VectorStore::delete_by_file(rel_path)` — LanceDB delete predicate | S | 3.6 | ⬜ |
| 3.10 | `SecretDetector` module — ~50 gitleaks regexes + Shannon-entropy check + file-level skip list (B1.2/B1.5) | M | 2.1 | ⬜ |
| 3.11 | `vektor models download [--lite]` subcommand — idempotent, resumable, progress bar (B6.1) | M | 3.2 | ⬜ |
| 3.12 | `v0.3.0` release tag + Phase 4 expansion | M | 3.7c, 3.10, 3.11 | ⬜ |

---

## Phase exit criteria

All must be true before tagging `v0.3.0`:

- [ ] All 12+ tasks above marked ✅ Done (note 3.7 will be split into ≥3 sub-tasks per the per-task files written by task 2.9)
- [ ] `vektor models download` produces `~/.vektor/models/jina-embeddings-v2-base-code/model.onnx` + `tokenizer.json`
- [ ] `vektor index <repo>` chunks files (Phase 2), embeds them (3.3), and writes to `~/.vektor/<project>/lance/` (3.7)
- [ ] Running `vektor index` twice on an unchanged repo skips re-embedding (chunk-level cache via content_hash works)
- [ ] Modifying one function in a 10-file repo re-embeds only the changed chunks, not the whole file (per PRD §4.5 fix)
- [ ] Planting an AWS key (`AKIAIOSFODNN7EXAMPLE`) in a test repo: indexing skips that chunk and logs a warning (B1.2 verified)
- [ ] `.env` file in the test repo is not even read into memory (B1.5 verified)
- [ ] Switching `config.embedding.backend` from `onnx` to `openai` with a valid API key: indexing succeeds via the cloud backend
- [ ] LanceDB table has the full schema from PRD §4.10 (id, content_hash, vector, rel_path, start_line, end_line, symbol_name, symbol_type, language, content, last_modified)
- [ ] First-query cold-start latency <5s due to ONNX warm-up
- [ ] CI green; tag pushed; release artifacts uploaded
- [ ] Phase 4 per-task files written before Phase 4 starts

---

## What v0.3.0 *intentionally does not* include

- BM25 / Tantivy indexing (Phase 4)
- RRF fusion or hybrid search (Phase 4)
- Real MCP tool handlers — `index_codebase` becomes real here, but `search_code` and `get_context_for_prompt` remain no-op stubs until Phase 4–5
- Ollama backend (Phase 5 — defer; OnnxEmbedder + OpenAI cover most users)
- Two-tier indexing (`ShallowIndexer` is Phase 5)

---

## Notes

- **Phase 2 handoff**: discovery, HashStore, chunking, and Phase 2 `vektor index` behavior exist locally. Phase 3 consumes those APIs after the `v0.2.0` publish gate is approved and per-task files are expanded.
- **ONNX integration is the hardest single task**. Reading the `ort 2.0.0-rc.12` docs and one working example before writing 3.2/3.3 saves multiple hours of debugging mean-pooling math.
- **L2 normalization is required for cosine-via-dot-product**: if vectors aren't unit-norm, the LanceDB ANN scores are meaningless. Verify by computing `vec.iter().map(|x| x*x).sum::<f32>().sqrt()` after normalization; must equal 1.0 ± 1e-5.
- **Don't pin arrow versions** (per PRD Section 11 fix): let `lancedb 0.29` own arrow's version transitively. If `cargo tree -d` shows duplicate arrow versions, fix at the consumer side — never patch lancedb.
- **`tokenizer.json` format**: HuggingFace's `tokenizers` crate handles BPE, WordPiece, and unigram tokenizers from the same JSON. No vocab loading needed beyond pointing at the file.
- **Content-addressed chunk IDs** (PRD §4.5): AST chunks use `sha256(rel_path:symbol_name:content_hash)`; sliding-window chunks use `sha256(rel_path:chunk_{ordinal}:content_hash)`. Phase 2 task 2.7 implemented this; Phase 3 just consumes it.
- **Chunk-level embedding cache**: the trick is to read existing chunks BEFORE deleting (PRD §4.5 step 1 before step 2). Get this ordering wrong and the "cache" hits zero times.

---

## When this phase completes

1. Mark all tasks ✅
2. Tag `v0.3.0`
3. Update Current State tables
4. Expand `phase-4-search-mcp/` from task list to per-task files (task 3.12 closes this)
