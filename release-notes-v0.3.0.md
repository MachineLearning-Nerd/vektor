# Vektor v0.3.0 — Embedding + Storage

## Summary

`v0.3.0` turns `vektor index` from a Phase 2 chunk-only pipeline into a full
embedding + vector-storage pipeline. Chunks are now embedded (local ONNX Jina v2
Base Code by default, or an OpenAI-compatible cloud backend) and written to an
embedded LanceDB table. Re-indexing reuses unchanged embeddings via a
content-addressed cache, and obvious secrets are skipped before they are ever
embedded. The `index_codebase` MCP tool is now real (vector-only) and shares the
exact same indexing core as the CLI.

This release still does not build BM25/Tantivy indexes, do hybrid/RRF search, or
enable the real `search_code` / `get_context_for_prompt` MCP handlers — those
land in Phase 4 and Phase 5.

This is a source-install, **notes-only** release. No binary assets are attached;
prebuilt binary release artifacts remain deferred to Phase 6 / task 6.2.

## What's new

- **`Embedder` trait** — async `embed` / `dim` / `name` / `prefix_for_document` /
  `prefix_for_query`, the common surface for all embedding backends.
- **`OnnxEmbedder`** — loads a local ONNX model + `tokenizer.json` from
  `~/.vektor/models/` via `ort` + `tokenizers`, runs a warm-up embed at
  construction, then tokenizes → runs the session → mean-pools → L2-normalizes,
  batching texts in groups of 32. Query/document prefixes are prepended per the
  Jina v2 convention.
- **`OpenAiCompatEmbedder`** — `POST /v1/embeddings` against any OpenAI-compatible
  endpoint, with exponential backoff on HTTP 429. The configured API key is
  redacted from `Debug` output.
- **`build_embedder(config)` factory** — selects the configured backend and
  returns `Box<dyn Embedder>`, with fallback-to-ONNX behavior.
- **`VectorStore` on LanceDB** — connects to `~/.vektor/<project>/lance/` and
  creates the `chunks` table with the full PRD §4.10 Arrow schema (`id`,
  `content_hash`, `vector`, `rel_path`, `start_line`, `end_line`, `symbol_name`,
  `symbol_type`, `language`, `content`, `last_modified`).
- **Content-addressed re-index cache** — `existing_embeddings_by_content_hash`
  reads existing chunk embeddings *before* deleting, so a re-run reuses unchanged
  embeddings keyed by `content_hash`. A one-function edit re-embeds only the
  changed chunks, not the whole file (per PRD §4.5).
- **Reindex reuse planning + `reindex_file`** — `plan_reindex` decides which
  chunks to reuse vs re-embed; `reindex_file` performs the read-before-delete →
  embed-new → insert flow and reports embedded/reused counts.
- **`VectorStore::search`** — ANN query via LanceDB `nearest_to` with an optional
  SQL-like filter predicate, returning ranked `SearchResult` rows.
- **`VectorStore::delete_by_file`** — deletes all chunks for an exact `rel_path`
  via a LanceDB delete predicate, with churn tracking.
- **Secret-aware indexing (`SecretDetector`)** — static gitleaks-style rules plus
  a Shannon-entropy check skip chunks that contain obvious secrets before
  embedding (B1.2). The discovery path also skips `.env*`, `*.pem`, `*.key`, and
  `credentials.json` files entirely — they are never read into memory (B1.5).
- **`vektor models download [--lite]`** — idempotent, resumable (HTTP `206`
  range-resume) download of Jina v2 (default) or bge-small (`--lite`) into
  `~/.vektor/models/`, with an `indicatif` progress bar (B6.1).
- **Real `index_codebase` MCP tool (vector-only)** — no longer a "not implemented"
  stub. It loads the default config and calls the same `crate::cli::index_path`
  core the CLI uses (no duplicated indexing loop), returning real stats
  (`files`, `changed`, `unchanged`, `failed`, `chunks`, `embeddings`, `reused`,
  `skipped_secrets`). It writes nothing to stdout (the stdio MCP channel) and
  returns a JSON error object instead of panicking on failure. Phase 4 task 4.6
  *extends* this same handler to also write the Tantivy BM25 index.
- **Error taxonomy** — `VektorError` gained `Embedding`, `Storage`, `Network`
  (and, in this release, `Download`) variants for clear, fail-fast diagnostics.

## Phase 3 exit criteria — verification status

Vektor's CI environment has no network access and no downloaded model, so the
model-dependent criteria are exercised by `#[ignore]`d smoke tests and must be
run manually after `vektor models download`. Everything else is verified by the
standard test suite.

| Exit criterion | Status | How verified |
|---|---|---|
| All 15 Phase 3 task files marked ✅ Done | ✅ Verified | Phase 3 README / DEPENDENCIES.md updated with commit hashes |
| `cargo fmt --check` | ✅ Verified | clean (exit 0) |
| `cargo clippy --workspace --all-targets -- -D warnings` | ✅ Verified | clean (exit 0) |
| `cargo test --workspace` | ✅ Verified | 224 passed, 0 failed, 7 ignored |
| LanceDB table has the full PRD §4.10 schema | ✅ Verified | `vector_store` schema + insert/search/delete unit tests |
| Re-run on unchanged repo skips re-embedding (content_hash cache) | ✅ Verified | `vector_store` reuse/`plan_reindex` tests + `index_cli` (model-gated for the real-model path) |
| One-function edit re-embeds only changed chunks | ✅ Verified | `vector_store` reindex-reuse tests with a fake embedder |
| AWS key (`AKIAIOSFODNN7EXAMPLE`) chunk is skipped + logged | ✅ Verified | `secrets` tests + `index_codebase` MCP test asserts `skipped_secrets >= 1` |
| `.env` file is never read into memory | ✅ Verified | discovery file-level skip-list tests (B1.5) |
| OpenAI-compatible backend path works | ✅ Verified | `openai` embedder tests against a `wiremock` mock server |
| `vektor models download` produces `model.onnx` + `tokenizer.json` | ⏳ Manual | `#[ignore]`d HF smoke tests — requires network. Run manually (see below) |
| `vektor index <repo>` writes LanceDB vectors end-to-end | ⏳ Manual | `#[ignore]`d `index_cli` tests — requires a downloaded ONNX model |
| First-query cold-start latency <5s (ONNX warm-up) | ⏳ Manual | warm-up implemented in `OnnxEmbedder::new`; latency measured manually after model download |
| Switching backend `onnx` → `openai` indexes via cloud | ⏳ Manual | unit-verified via mock; full E2E with a real key is a manual check |
| Phase 4 per-task files written before Phase 4 starts | ✅ Verified | `docs/plans/initial/phase-4-search-mcp/01-..08-*.md` |
| CI green; tag pushed; notes-only GitHub Release | ⏳ Deferred | publication is a separate, human-authorized step (see Release steps) |

### Running the model-dependent checks manually

After a model download, run the ignored smoke + end-to-end tests:

```bash
# Download the model into an isolated home (so it doesn't touch ~/.vektor)
FAKE_HOME=$(mktemp -d)
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" cargo run --release -- models download --lite

# Real-model unit smoke tests (ONNX load + embed; HF download smoke)
cargo test models_download -- --ignored --nocapture
cargo test onnx_embedder -- --ignored --nocapture

# End-to-end index against a downloaded model
cargo test --test index_cli -- --ignored --nocapture

# Full manual E2E: index this repo's src/ twice (second run reuses embeddings)
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" ./target/release/vektor index src/
HOME="$FAKE_HOME" USERPROFILE="$FAKE_HOME" ./target/release/vektor index src/
```

## Install

Prerequisites: Rust 1.91+ and `protoc` (macOS: `brew install protobuf`; Linux:
`apt-get install protobuf-compiler` or `dnf install protobuf-compiler`).
`protoc` is required because the LanceDB/Arrow dependency stack invokes it during
compilation.

```bash
cargo install --git https://github.com/MachineLearning-Nerd/vektor --tag v0.3.0 --locked
vektor models download   # fetch Jina v2 (or `--lite` for bge-small) before indexing
```

## Deferred to later phases

- BM25 / Tantivy indexing, RRF fusion, and hybrid search — Phase 4.
- Real `search_code` MCP handler — Phase 4.
- Real `get_context_for_prompt` MCP handler + full context assembly — Phase 5.
- Ollama backend — Phase 5.
- Two-tier (shallow) indexing — Phase 5.
- Prebuilt binary release artifacts (signed binaries + checksums + model tarballs)
  — `v0.4.0` / task 6.2 (this release is notes-only).

## Known minor items

- The `model_name == hf_repo` duplication in the `ModelSpec` table for the two
  built-in models is a harmless DRY nit; left as-is.

## Release steps (DO NOT run without explicit authorization)

This repository is **private and stays private until launch (`v0.4.0`)**. The
following steps are an irreversible, outward-facing publication sequence and must
only be executed after the maintainer explicitly authorizes them in-session.
They are documented here, not performed by task 3.12.

1. Confirm the working tree is clean after the `v0.3.0` release commit.
2. Push the release commit to `origin/phase-3-embedding-storage` (and merge to the
   intended release branch per the project's branching choice).
3. Wait for the `ci.yml` run on that exact commit to conclude `success` on
   macOS-aarch64 + linux-x86_64.
4. Create and push the annotated tag:
   ```bash
   git tag -a v0.3.0 -m "Vektor v0.3.0 — Embedding + Storage"
   git push origin v0.3.0
   ```
5. Create the **private, notes-only** GitHub Release with this file as the body
   and **zero** assets:
   ```bash
   gh release create v0.3.0 --title "v0.3.0 — Embedding + Storage" \
     --notes-file release-notes-v0.3.0.md --verify-tag
   ```
6. Do **not** flip repository visibility. The public flip remains Phase 6.6 /
   `v0.4.0` only and is a separate one-way action requiring its own authorization.
