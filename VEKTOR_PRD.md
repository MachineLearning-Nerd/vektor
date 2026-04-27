# Vektor — Product Requirements Document

> **Local-first · Real-time · Zero cloud dependency**
> Codebase **context engine** MCP server built in Rust.
> Not just search — intelligent context assembly for AI coding agents.
> Matches Augment Code Context Engine · 100% your machine · 100% open source.

---

## Document Info

| Field | Value |
|---|---|
| Project | Vektor |
| Version | 2.3.0 |
| Status | v2.3 — Context Engine + Agent DX + Accuracy/Performance + Database Architecture + Retrieval Accuracy hardening |
| Stack | Rust + ONNX Runtime + LanceDB (embedded) + Tantivy + MCP Protocol |
| License | MIT (open source) |
| Author | Dinesh |
| Target User | Individual developers using Claude Code, Cursor, Codex CLI |
| Goal | Production-grade context engine. Learn as you build. |

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Problem Statement](#2-problem-statement)
3. [Goals and Non-Goals](#3-goals-and-non-goals)
4. [System Architecture](#4-system-architecture)
5. [Context Assembly Layer](#5-context-assembly-layer)
6. [Embedding Backend Strategy](#6-embedding-backend-strategy)
7. [Agent DX & Adoption Strategy](#7-agent-dx--adoption-strategy)
8. [MCP Tools API](#8-mcp-tools-api)
9. [Performance Targets](#9-performance-targets)
10. [Technology Stack](#10-technology-stack)
11. [Phased Implementation Plan](#11-phased-implementation-plan)
12. [Risks and Mitigations](#12-risks-and-mitigations)
13. [Success Metrics](#13-success-metrics)
14. [Implementation Rules](#14-implementation-rules)

---

## 1. Executive Summary

Vektor is a high-performance, local-first **codebase context engine** exposed as a Model Context
Protocol (MCP) server. Built entirely in Rust, it gives Claude Code, Codex CLI, Cursor, and any
MCP-compatible AI coding assistant the ability to deeply understand an entire codebase —
without sending code to the cloud, without requiring external API keys for core operation,
and with real-time sub-500ms re-indexing triggered by OS file-save events.

**The one-line pitch:**
> *"Augment Code's Context Engine, but open source, local-first, and in Rust."*

**Search engine vs Context engine:**
A search engine returns ranked results. A context engine assembles **token-budgeted, deduplicated,
relationship-aware context packages** optimized for LLM consumption. Vektor is the latter.
Its killer feature is `get_context_for_prompt` — an MCP tool that lets any AI agent say
"give me the best context about authentication that fits in 8K tokens" and receive a curated
package of code, docs, and related files ready for reasoning.

**The problem it solves:**
Claude Code has no persistent memory of your codebase. Every session starts from zero.
Engineers burn tokens just discovering where relevant code lives. Vektor fixes this by
maintaining a live, semantic index of your entire codebase — always up to date, always local,
always fast — and delivering that context in exactly the right quantity and format for AI agents.

**Why open source matters:**
Augment Code charges $50+/seat/month and sends your code to Google Cloud. Vektor gives you
the same capabilities for free, on your machine, with full transparency into how context
is assembled. Developers can customize chunking strategies, embedding models, and context
assembly logic for their specific needs.

**Key differentiators vs competition (v2.1):**
- **Two-tier indexing**: Keyword search available in <5s, full semantic index builds in background. Never blocks tool responses on indexing completion.
- **Code-optimized embeddings**: Default model is Jina Embeddings v2 Base Code (768d, Apache 2.0) — 20-50% better retrieval vs general-purpose models.
- **Context assembly layer**: Token-budgeted, deduplicated, relationship-aware packages. CocoIndex Code (nearest open-source competitor) has no equivalent — it's search-only.
- **Zero-config agent adoption**: Skills integration (SKILL.md), MCP server instructions, and first-run auto-indexing mean agents discover and use Vektor without manual setup.
- **Explicit `token_budget`**: Augment does NOT expose this parameter to agents — Vektor gives agents direct control over context size.

---

## 2. Problem Statement

### 2.1 The Context Window Problem

Claude Code and Codex CLI have no persistent knowledge of a codebase. Every session starts
from zero. Engineers must manually provide context — pasting file contents, writing CLAUDE.md
docs, or relying on Claude's grep/find tools which burn massive tokens just to discover where
relevant code lives.

### 2.2 Current Tool Gaps

| Problem | Current Workaround | Cost |
|---|---|---|
| No codebase memory | Paste files manually each session | High token burn, slow start |
| Keyword search only | grep/find via Claude tools | Misses semantic relationships |
| No dependency tracking | Engineer mentally maps imports | Missed side-effects in refactors |
| Cloud embedding APIs required | OpenAI/VoyageAI API keys | Cost + latency + data privacy risk |
| No real-time updates | Re-index entire codebase on change | Minutes of delay, stale results |
| No project isolation | Single shared namespace | Collisions across projects |

### 2.3 Why Existing Solutions Fall Short

**Zilliz Claude Context MCP**
- Requires Milvus Cloud account + OpenAI API key — code leaves your machine
- No real-time file watching — you must manually trigger re-index
- No dependency graph
- ~40% token reduction claimed, but cloud round-trip adds 400ms+ latency per query

**Augment Code**
- Excellent architecture, but closed-source SaaS at $50+/seat/month
- No self-hosting option
- Proprietary context engine — no transparency into how it works

**code-memory (Python)**
- Good local option but Python GIL limits true parallelism
- No real-time file watching
- No dependency graph
- Slower indexing than Rust equivalent (roughly 3–5x)

**CocoIndex Code** (direct open-source competitor)
- Python wrapper over Rust engine, SQLite + sqlite-vec for vector storage
- Single `search` MCP tool — no context assembly, no token budgeting, no deduplication
- Text-based RecursiveSplitter despite "AST" marketing — no tree-sitter structural awareness
- No hybrid search (vector-only), no git history, no dependency graph
- 918 GitHub stars in 6 weeks — validates market demand, good DX (Skills integration, zero-config)
- Architecture ceiling is low: search-only, no context engine layer. Vektor surpasses it on every technical dimension.

**Sourcegraph Cody**
- Repo-level Semantic Graph (RSG) with Expand-and-Refine traversal
- 35% retrieval failure reduction with hybrid dense-sparse search
- Multi-hop graph expansion for complex queries (function → caller → caller's tests)
- Enterprise complexity — requires Sourcegraph server, not truly local-first
- Key innovation to adopt: graph-based code importance ranking (PageRank-style weighting)

**Cursor**
- Merkle tree for change detection — smarter than per-file SHA-256 hashing
- Content-addressed embedding cache: skip re-embedding unchanged chunks (not just unchanged files)
- Cross-user index sharing, custom embedding model trained on code
- PR history indexing with LLM summaries for Context Lineage
- Cloud-dependent ($20/month), proprietary — not local-first
- Key innovation to adopt: Merkle tree incremental sync as Phase 3 enhancement to HashStore

**Greptile**
- Code knowledge graph: functions/classes as nodes, dependencies as directed edges
- Multi-hop investigation for code review (e.g., "what breaks if I change this function?")
- Confidence scores on every result — agents can decide whether to trust low-confidence results
- Cloud-only API, not local-first
- Key innovation to adopt: confidence scores on search results

---

## 3. Goals and Non-Goals

### 3.1 Goals

| ID | Goal | Priority | Phase |
|---|---|---|---|
| G1 | Real-time file watching — re-index changed files in <500ms on save | Critical | 2 |
| G2 | Hybrid BM25 + dense vector search with RRF fusion | Critical | 1 |
| G3 | AST-aware chunking (tree-sitter) for 20+ languages | Critical | 1 |
| G4 | Pluggable embedding: local ONNX, OpenAI-compatible API, Ollama | Critical | 1 |
| G5 | Local vector database (LanceDB embedded) — zero cloud required | Critical | 1 |
| G6 | Incremental indexing via SHA-256 file hash diffing | High | 1 |
| G7 | **`get_context_for_prompt` — token-budgeted context assembly** | **Critical** | **1** |
| G8 | **Context deduplication, ranking, and LLM-optimized formatting** | **Critical** | **1** |
| G9 | **Related-file expansion (imports, tests, configs)** | **High** | **1** |
| G10 | **Query result caching (LRU)** | **High** | **1** |
| G11 | Dependency graph (import/export tracking per file) | High | 2 |
| G12 | Multi-project isolation with per-project collections | High | 2 |
| G13 | MCP-compliant server (stdio + SSE transport) | Critical | 1 |
| G14 | Single compiled Rust binary — zero runtime dependencies | High | 1 |
| G15 | Symbol-level search (find function by name across codebase) | Medium | 2 |
| G16 | Git history indexing — commit messages + changed files searchable | Medium | 2 |
| G17 | Workspace-aware — index docs (README, *.md) alongside code | Medium | 1 |
| G18 | Precision@5 benchmark: beat Zilliz MCP retrieval accuracy | High | 3 |
| G19 | Two-tier indexing: keyword search available in <5s, semantic in background | Critical | 1 |
| G20 | Skills integration (SKILL.md) for zero-config agent adoption | High | 1 |
| G21 | Code-optimized default embedding (Jina v2 Base Code, 768d) | Critical | 1 |
| G22 | Recency-weighted ranking (recently modified files ranked higher) | High | 1 |
| G23 | Context quality feedback tool (`report_context_quality`) | Medium | 2 |
| G24 | Progressive context delivery for large token budgets | Medium | 2 |
| G25 | Codestral Embed / Voyage Code 3 as cloud embedding options | Medium | 2 |
| G26 | Content-addressed chunk IDs (stable across line movements) | Critical | 1 |
| G27 | Delete-then-insert re-indexing (zero orphaned chunks) | Critical | 1 |
| G28 | Static synonym expansion for BM25 (10-15% recall improvement) | High | 1 |
| G29 | Adaptive hybrid search weights (identifier vs natural language queries) | High | 1 |
| G30 | Graceful shutdown with flush-and-close sequence | High | 1 |
| G31 | ONNX warm-up at server startup (eliminate cold-start latency) | High | 1 |
| G32 | Chunk-level embedding cache (skip unchanged chunk re-embedding) | High | 1 |

### 3.2 Non-Goals

- Not a full IDE — no autocomplete, no inline suggestions, no UI
- Not a replacement for LSP (Language Server Protocol)
- Not a cloud service — intentionally local-first by design
- Not a training data pipeline — indexes for retrieval only
- Not cross-machine sync — single developer machine scope for Phase 1

---

## 4. System Architecture

### 4.1 High-Level Design

```
Claude Code / Codex CLI / Cursor / Any MCP Client
          ↕  MCP Protocol (stdio / SSE)
┌──────────────────────────────────────────────────────┐
│                 Vektor (Rust Binary)                  │
├──────────────────────────────────────────────────────┤
│  MCP Server (rmcp)  →  Tool Handlers (8 tools)       │
├──────────────────────────────────────────────────────┤
│         ★ Context Assembly Layer ★                    │
│  ┌──────────┬───────────┬──────────┬───────────┐     │
│  │ Assembler│ Dedup     │ Expander │   Cache   │     │
│  │ (budget) │ (overlap) │ (related)│   (LRU)   │     │
│  └──────────┴───────────┴──────────┴───────────┘     │
├──────────┬──────────┬───────────┬────────────────────┤
│  Watcher │ Chunker  │ Embedder  │     Searcher       │
│ (notify) │(tree-sitter)│ (ONNX/  │  (Hybrid RRF)    │
│          │          │ OpenAI/  │  BM25 + Semantic    │
│          │          │  Ollama) │                     │
└────┬─────┴────┬─────┴─────┬────┴──────────┬─────────┘
     ↓          ↓           ↓               ↓
┌──────────────────────────────────────────────────────┐
│  LanceDB (embedded)  +  Tantivy (BM25)  +  SQLite    │
│  ~/.vektor/{project_hash}/  (persisted)               │
└──────────────────────────────────────────────────────┘
```

### 4.2 Subsystems

#### Watcher (`notify` crate)
Listens for OS-native inode events:
- macOS → FSEvents
- Linux → inotify
- Windows → ReadDirectoryChangesW

On file-save event: debounce 200ms → SHA-256 hash → if changed → queue for incremental
re-index. Target: <500ms from save to searchable.

**Phase 3 enhancement:** Replace per-file SHA-256 hashing with a **Merkle tree** approach
(inspired by Cursor). A content-addressed tree enables chunk-level change detection — skip
re-embedding unchanged chunks even within a changed file. This reduces incremental indexing
cost by ~60% for typical edits.

#### Chunker (`tree-sitter`)
Parses source files into Abstract Syntax Trees. Extracts function / class / method / struct
nodes as discrete chunks with:
- Symbol name
- Line range (start_line, end_line)
- Docstring / comment context prepended
- Language tag

**Max chunk size (v2.2):** AST chunks exceeding `chunk_max_lines` (default: 200 lines) are
sub-chunked with 25% overlap. This prevents large functions (300+ lines) from producing
single chunks that waste token budget and degrade embedding quality. Rust `impl` blocks
and Python classes are chunked at the method level, not the container level.

**Header preservation (v2.3):** When sub-chunking large AST nodes, **always prepend the
parent node's signature** (first 3-5 lines: function signature, class declaration, etc.)
to each sub-chunk. Without this, the second sub-chunk of a 300-line function is semantically
orphaned — the embedding cannot capture what function this code belongs to. This produces
a 15-20% retrieval improvement for large functions (validated by Aider's chunking experiments).

Falls back to sliding window for unsupported file types:
- **Code files**: 80-line window, 25% overlap (20 lines)
- **Documentation files** (*.md, *.txt, *.rst): 40-line window, 40% overlap (16 lines)
  — smaller windows match paragraph-level structure, higher overlap improves recall at
  section boundaries

Supported languages (Phase 1): Python, TypeScript, JavaScript, Rust, Go
Extended (Phase 3): Java, C/C++, C#, Ruby, PHP, Swift, Kotlin, Scala

#### Embedder (`ort` crate / HTTP)
Pluggable backend behind a common Rust trait:

```rust
#[async_trait]
trait Embedder: Send + Sync {
    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>>;
    fn dim(&self) -> usize;
    fn name(&self) -> &str;
    fn prefix_for_document(&self) -> &str { "" }  // e.g., "search_document: " for Jina v2
    fn prefix_for_query(&self) -> &str { "" }      // e.g., "search_query: " for Jina v2
}
```

**Warm-up (v2.2):** On `vektor serve` startup, embed a dummy string to initialize the ONNX
session. This avoids a 3-5 second cold start on the first real query. Warm-up runs during
server initialization before accepting MCP tool calls.

Backends: LocalOnnx, OpenAiCompat, Ollama (see Section 6).

#### Vector Store (LanceDB embedded)
LanceDB runs in-process as a true embedded database — no separate server, no Docker, no sidecar.
Built on Apache Arrow + Lance columnar format for efficient vector operations.
- Data persists to `~/.vektor/{project_hash}/lance/`
- Each project gets an isolated LanceDB table
- ANN vector search with automatic index building
- Rich metadata filtering via SQL-like predicates during search (not post-retrieval)
- Upsert via delete-then-insert per file (see Section 4.5)
- Scales to 100K+ chunks with sub-10ms search latency
- Explicit ANN index management: build `IVF_PQ` index after initial indexing, rebuild on chunk churn

**ANN Index Rebuild Strategy (v2.2.1):**
- After initial indexing: build IVF_PQ with `nlist=sqrt(N)`, `nprobe=nlist/4`
- On every `reindex_file`: increment `chunks_inserted_since` and `chunks_deleted_since` in `index_stats` table (see Section 4.10)
- Check trigger: if `(chunks_inserted_since + chunks_deleted_since) > chunks_at_last_ann_rebuild * 0.1`:
    → Rebuild IVF_PQ index in background
    → Reset counters, update `chunks_at_last_ann_rebuild`
- Rebuild is non-blocking: queries use old index until new one is ready
- For <50K chunks: skip IVF_PQ entirely (brute-force scan is <10ms)

Why LanceDB over Qdrant: The `qdrant-client` Rust crate is a gRPC client that requires a
running Qdrant server — it has no true embedded mode in Rust (unlike the Python client).
LanceDB is genuinely embedded: single process, zero network, zero setup.

#### Full-Text Index (Tantivy)
Pure-Rust Lucene equivalent. Used for BM25 keyword search:
- Symbol names
- Identifiers
- String literals
- Comments

Runs in same process as LanceDB — no separate service.

**Tantivy Schema (v2.2.1):**
```
Fields:
  - chunk_id    (Utf8, STORED, not indexed) — matches LanceDB id for cross-store joins
  - rel_path    (Utf8, STORED + STRING, not tokenized) — exact match filtering
  - content     (Utf8, TEXT, tokenized with en_stem) — BM25 full-text search
  - symbol_name (Utf8, TEXT, tokenized + STORED) — boosted in BM25 scoring
  - language    (Utf8, STRING, STORED) — exact match filtering
  - start_line  (u64, STORED) — for result display
  - end_line    (u64, STORED) — for result display
  - index_depth (Utf8, STRING, STORED) — "shallow" | "deep" for transition scoring

BM25 field boosting:
  - symbol_name: 2.0x boost (function names are high-signal for identifier queries)
  - content: 1.0x (default)
```

The `index_depth` field enables distinguishing shallow vs deep results during the
two-tier transition period (Section 4.6), addressing scoring comparability between
shallow keyword-only entries and deep AST-chunked entries.

#### Hybrid Search + RRF Fusion
Combines Tantivy BM25 + LanceDB semantic results via Reciprocal Rank Fusion:

```
rrf_score(d) = Σ 1 / (k + rank_i(d))

where k = 60 (standard RRF constant)
```

**Adaptive weight selection (v2.2, refined v2.2.1):** Instead of fixed 0.6/0.4 weights,
detect query type using **density-based classification** (not binary detection):
- Count identifier tokens (matching `[a-z]+_[a-z]+`, `[a-z]+[A-Z]`, or `\w+\.\w+` patterns)
- Compute identifier density: `identifier_tokens / total_tokens`
- **>60% identifier density**: `semantic=0.4, keyword=0.6` (e.g., "validate_token AuthMiddleware")
- **<25% identifier density**: `semantic=0.7, keyword=0.3` (e.g., "how does authentication work")
- **25-60% (mixed queries)**: `semantic=0.6, keyword=0.4` (e.g., "how does validate_token handle expired JWTs")
- This prevents a single identifier in a natural language question from flipping to BM25-heavy weights

**Static synonym expansion (v2.2):** Before BM25 search, expand query terms using a built-in
synonym map for common code concepts (~50 entries). Example:
```
"auth"  → ["auth", "authentication", "authorize", "login", "session", "token", "jwt", "bearer", "oauth"]
"db"    → ["db", "database", "query", "sql", "migration", "schema", "table"]
"api"   → ["api", "endpoint", "route", "handler", "controller", "request", "response"]
"config"→ ["config", "configuration", "settings", "options", "env", "environment"]
```
BM25 query is expanded to OR across all synonyms. Semantic search uses the original query only.
This closes a 10-15% recall gap on natural language queries with minimal latency cost (~3-8ms
for the expanded BM25 query, depending on synonym count and index size).

Optional re-ranking via cross-encoder model (Phase 2, moved from Phase 3 in v2.3).

### 4.3 Data Flow

```
File saved on disk
  → notify event fires
  → debounce 200ms (batch all changes in window)
  → for each changed file in batch:
      → SHA-256 hash compared to stored hash
      → if changed:
          → read file content
          → tree-sitter AST parse
          → extract chunks, compute content_hash per chunk
          → query LanceDB for existing chunks → HashMap<content_hash, Vec<f32>> (v2.2.1)
          → compare content_hashes: identify changed vs unchanged chunks
          → embed ONLY changed chunks (local ONNX or cloud API)
          → delete_by_file on LanceDB (remove old chunks)
          → insert ALL chunks into LanceDB (reused embeddings + new embeddings)
          → delete_by_file on Tantivy (remove old entries)
          → insert new Tantivy entries
          → store new hash in SQLite state
  → batch Tantivy commit (once per debounce window, not per file) (v2.2)
  → ready: search reflects new content
```

### 4.4 Two-Tier Indexing Strategy

First-time indexing must never block tool responses. Vektor uses a two-tier approach:

```
Phase 1: Shallow Index (0-5s)
  → Walk directory tree, build file list
  → Build Tantivy BM25 index from file paths + first 50 lines of each file
  → search_code available immediately (keyword mode only)
  → get_context_for_prompt returns partial results with index_status: "partial"

Phase 2: Deep Index (background, 30s-5min depending on codebase size)
  → Chunk files with tree-sitter (AST-aware)
  → Compute embeddings via configured backend (ONNX/cloud)
  → Build LanceDB vector store
  → search_code now supports hybrid/semantic modes
  → get_context_for_prompt returns full-quality results with index_status: "full"
```

### 4.5 Re-indexing Strategy: Delete-Then-Insert

**v2.2 fix:** The previous upsert-only approach via `merge_insert` left orphaned chunks when
functions were removed or files deleted. Vektor now uses **delete-then-insert per file**:

```
When re-indexing file X:
  1. Query LanceDB for existing chunks by rel_path="X"
     → hold in memory as HashMap<content_hash, Vec<f32>> (~46KB per file)
  2. delete_by_file("X") on both LanceDB AND Tantivy
  3. Chunk file X with tree-sitter, compute content_hash per chunk
  4. For each new chunk: if content_hash exists in the HashMap → reuse embedding vector
     Otherwise → embed via ONNX/cloud (only changed chunks get embedded)
  5. Insert ALL chunks (reused + newly embedded) into LanceDB + Tantivy
  6. Update HashStore with new file hash
```

This guarantees zero orphaned chunks. If the process crashes between steps 2 and 6,
the HashStore still shows `pending` for file X, and recovery re-processes from step 1.

**Important:** Step 1 (read existing embeddings) MUST happen BEFORE step 2 (delete).
Without this ordering, the embedding cache has nothing to compare against.

**Chunk ID scheme (v2.2 fix):**
Chunk IDs are now content-addressed:
- For AST chunks: `sha256(rel_path + ":" + symbol_name + ":" + content_hash)`
- For sliding-window chunks (no symbol_name): `sha256(rel_path + ":chunk_" + ordinal_index + ":" + content_hash)`
  — the ordinal index (0, 1, 2...) disambiguates identical content blocks in the same file,
  preventing silent overwrites in config files and generated code (v2.2.1 fix)

This means:
- If a function moves lines but content is unchanged → same chunk ID → skip re-embedding
- If content changes → new chunk ID → re-embed
- Feedback keyed on `(rel_path, symbol_name)` survives re-indexing (not tied to line numbers)
- For sliding-window chunks, feedback keyed on `(rel_path, start_line_bucket)` where
  `start_line_bucket = start_line / chunk_size` — stable across minor line shifts (v2.2.1 fix)

**Chunk-level embedding cache (v2.2):**
The read-before-delete pattern in step 1 enables chunk-level caching. For a file with 8 chunks
where 1 changed: 7 embeddings are reused from the HashMap, 1 is freshly computed. This reduces
re-embedding by 5-10x for typical single-function edits.

**File deletion handling:**
When the watcher detects a file deletion event, call `delete_by_file` on LanceDB, Tantivy,
and HashStore. No orphaned chunks remain.

**Key principle:** Never return an error because indexing is incomplete. Degrade gracefully:
- No index → trigger shallow index, return empty results with `index_status: "building"`
- Shallow index only → return keyword results with `index_status: "partial"`
- Full index → return hybrid results with `index_status: "full"`

All tool responses include `index_status` and `index_coverage_pct` so agents can
inform users about context quality and decide whether to wait for full indexing.

### 4.6 Shallow-to-Deep Transition Strategy (v2.2)

When deep indexing completes for a file, the shallow Tantivy entry must be replaced:
1. Delete the shallow Tantivy document for that file (matched by `rel_path`)
2. Insert the deep Tantivy documents (one per chunk) for that file
3. This happens per-file, not as a bulk swap — transition is gradual
4. During transition, some files have shallow entries and others have deep entries
5. Results from deep-indexed files will have richer content matches

### 4.7 Embedding Dimension Migration (v2.2)

Switching embedding models (e.g., Jina v2 768d → bge-small 384d) changes vector dimensions.
LanceDB tables are dimension-locked — you cannot mix 384d and 768d vectors.

**Strategy:**
- Store model info in `project_metadata` table (see Section 4.10):
  ```
  ("model_name", "jinaai/jina-embeddings-v2-base-code")
  ("embedding_dim", "768")
  ("last_full_index_at", "1710756000")
  ("vektor_version", "0.1.0")
  ```
- On startup, compare configured model vs stored `model_name` and `embedding_dim`
- If mismatch detected: warn user, trigger full re-index with `clear_index`
- Clear message: "Embedding model changed (768d → 384d). Re-indexing required."
- Feedback data in SQLite is preserved (keyed on `rel_path + symbol_name`, not vectors)

### 4.8 Graceful Shutdown (v2.2)

The MCP server must handle `SIGTERM`/`SIGINT` cleanly to avoid the crash recovery path on
every normal shutdown:

```
1. Catch shutdown signal (tokio::signal)
2. Stop accepting new MCP tool calls
3. Wait for in-flight operations to complete (5s timeout)
4. Flush pending Tantivy commits
5. Flush pending SQLite writes (HashStore, FeedbackStore)
6. Close LanceDB connections
7. Exit cleanly
```

Without this, every server stop triggers HashStore crash recovery on next startup.

### 4.9 Cross-Store Consistency (v2.2)

Three storage engines (LanceDB, Tantivy, SQLite) mean three separate commit semantics.
If the process crashes mid-write:

**Recovery strategy:**
- HashStore is the source of truth for file processing state
- On restart, files in `pending` status get full re-processing via delete-then-insert
- `delete_by_file` is called on LanceDB and Tantivy BEFORE re-inserting, ensuring a clean slate
- This is idempotent — safe to run multiple times on the same file

**SQLite WAL mode:** All SQLite databases (HashStore, FeedbackStore, DependencyGraph) use
WAL mode explicitly for concurrent reader/writer support. Each concern uses a separate
SQLite file to eliminate cross-concern write contention:
- `~/.vektor/{hash}/state.db` — HashStore
- `~/.vektor/{hash}/feedback.db` — FeedbackStore
- `~/.vektor/{hash}/deps.db` — DependencyGraph

### 4.10 Storage Schema Definitions

Formal schemas for all three SQLite databases and the Tantivy/LanceDB stores.
Every table, index, and field is defined here to eliminate implementation ambiguity.

#### SQLite: `state.db` (HashStore)

```sql
CREATE TABLE file_hashes (
    rel_path    TEXT PRIMARY KEY,
    hash        TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending', 'indexed', 'failed')),
    indexed_at  INTEGER  -- Unix epoch
);
CREATE INDEX idx_file_hashes_status ON file_hashes(status);
-- Used by: get_pending() for crash recovery, filtering by status

CREATE TABLE project_metadata (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- Stores: model_name, embedding_dim, last_full_index_at, vektor_version
-- Used by: dimension migration check (Section 4.7), ANN churn tracking

CREATE TABLE index_stats (
    stat_name  TEXT PRIMARY KEY,
    stat_value INTEGER NOT NULL DEFAULT 0
);
-- Tracks: chunks_at_last_ann_rebuild, chunks_inserted_since, chunks_deleted_since
-- Used by: ANN index churn tracking (10% threshold trigger)
```

#### SQLite: `feedback.db` (FeedbackStore)

```sql
CREATE TABLE feedback (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    rel_path   TEXT NOT NULL,
    chunk_key  TEXT NOT NULL,          -- symbol_name or "line_bucket_{N}"
    useful     INTEGER NOT NULL        -- 1 = useful, 0 = irrelevant
                CHECK (useful IN (0, 1)),
    timestamp  INTEGER NOT NULL        -- Unix epoch
);
CREATE INDEX idx_feedback_lookup ON feedback(rel_path, chunk_key);
-- Used by: get_adjustment() — compound index covers the primary query pattern
CREATE INDEX idx_feedback_timestamp ON feedback(timestamp);
-- Used by: 30-day exponential decay pruning query
```

#### SQLite: `deps.db` (DependencyGraph)

```sql
CREATE TABLE deps (
    from_file TEXT NOT NULL,
    to_file   TEXT NOT NULL,
    PRIMARY KEY (from_file, to_file)
);
CREATE INDEX idx_deps_reverse ON deps(to_file, from_file);
-- Used by: query(file, Direction::ImportedBy) — reverse lookup
-- Without this index, "who imports this file" queries do full table scan
```

#### SQLite Maintenance Strategy

- **VACUUM:** Run on each SQLite file after `clear_index` (when bulk data is deleted)
- **WAL checkpoint:** Auto-checkpoint at 1000 pages (SQLite default is sufficient)
- **Pruning:** Delete feedback entries older than 90 days on server startup (3 half-lives)
- **Stats:** No explicit ANALYZE needed — SQLite auto-updates stats on small tables

---

## 5. Context Assembly Layer

> **This is what separates a search engine from a context engine.**
> Augment Code's 30-80% agent performance improvement comes from this layer, not from better
> embeddings. Vektor must get this right.

### 5.1 The Problem with Raw Search

Raw search returns ranked results. But AI agents need **curated context packages**:

```
Raw search output:              What agents actually need:
─────────────────               ────────────────────────
8 ranked snippets               Deduplicated, non-overlapping chunks
Unknown total tokens             Fits within agent's token budget
No related files                 Includes imports, tests, configs
Overlapping chunks               Formatted for LLM consumption
No quality floor                 Minimum relevance threshold
```

### 5.2 Architecture

```
Agent query: "how does auth work" + token_budget=8000
  ↓
┌─────────────────────────────────────────────────┐
│           Context Assembly Pipeline              │
├─────────┬──────────┬────────────┬───────────────┤
│ Search  │ Expand   │ Assemble   │ Budget        │
│ (hybrid)│ (related)│ (dedup+    │ (fit to       │
│         │          │  format)   │  token limit) │
└────┬────┴────┬─────┴─────┬──────┴──────┬────────┘
     ↓         ↓           ↓             ↓
  LanceDB  DependencyGraph  Deduplicator  TokenCounter
  Tantivy  FileProximity    Formatter     Allocator
```

### 5.3 Core Components

#### TokenCounter
Estimates token count for text using **language-specific ratios** (v2.3 fix):
- Python, JavaScript, TypeScript: `bytes / 3.5` (~5% error)
- Rust, Go, Java: `bytes / 4.2` (generics/type annotations increase bytes-per-token)
- Documentation (Markdown, RST): `bytes / 4.8` (natural language is more token-efficient)
- Default (unknown language): `bytes / 3.8`

The original `bytes / 3.5` heuristic had 29-43% error for non-Python code (Rust generics,
verbose Java types), causing significant budget over/under-fill.

**Two-pass budget verification (v2.3):** During greedy allocation, fill to 90% of budget
using the fast heuristic. Then run a precise token count via `tiktoken-rs` on the assembled
package. If under budget, add more chunks. If over, truncate the last chunk. This two-pass
approach adds <1ms and ensures accurate budget utilization.

#### ContextAssembler
The orchestrator. Given search results + config, produces a `ContextPackage`:

```rust
pub struct AssemblyConfig {
    pub token_budget: usize,         // max tokens in output (e.g., 8000)
    pub max_files: usize,            // max distinct files to include (e.g., 10)
    pub include_related: bool,       // expand to related files?
    pub min_relevance: f32,          // quality floor (e.g., 0.5)
    pub deduplicate: bool,           // remove overlapping chunks?
    pub include_docs: bool,          // include README/docs if relevant?
    pub scope: Option<String>,       // workspace-relative path for monorepo scoping (v2.3)
}

pub struct ContextPackage {
    pub chunks: Vec<ContextChunk>,   // deduplicated, ranked chunks
    pub files_included: Vec<String>, // all files represented
    pub total_tokens: usize,         // actual token count
    pub budget_used_pct: f32,        // how much of budget was used
    pub search_metadata: SearchMeta, // timing, scores, cache hit
    pub result_confidence: Confidence, // high/medium/low (v2.3)
    pub budget_gap_reason: Option<GapReason>, // why budget wasn't fully used (v2.3)
    pub suggested_action: Option<String>,     // agent guidance (v2.3)
    pub clusters: Option<Vec<ResultCluster>>, // for ambiguous queries (v2.3)
}

/// Confidence signaling (v2.3) — lets agents decide whether to trust results
pub enum Confidence {
    High,   // top result score >0.8, ≥3 results above min_relevance
    Medium, // top result 0.5-0.8
    Low,    // top result <0.5 or <2 results above threshold
}

pub enum GapReason {
    NoMoreRelevant,     // remaining chunks below min_relevance
    IndexIncomplete,    // index_status != "full"
    ThresholdFiltered,  // chunks exist but below min_relevance
}

/// Result clustering for ambiguous queries (v2.3)
pub struct ResultCluster {
    pub path_prefix: String,  // e.g., "src/auth/oauth"
    pub chunk_count: usize,
    pub avg_relevance: f32,
}

pub struct ContextChunk {
    pub content: String,             // chunk text
    pub rel_path: String,
    pub lines: (usize, usize),      // start_line, end_line
    pub symbol: Option<String>,
    pub relevance_score: f32,        // combined RRF score
    pub source: ChunkSource,         // Search | Related | Dependency
}
```

#### Deduplicator
When multiple chunks overlap (common with sliding window fallback), deduplicate by:
1. Sort chunks by file path + start_line
2. Check overlap ratio: only merge if overlap exceeds **50%** of the smaller chunk's line range
   — this prevents merging semantically distinct co-located functions (v2.2 fix)
3. When merging: keep the higher-scoring chunk, extend line range to cover both
4. Content is carried from search results (already in memory), NOT re-read from disk

#### RelatedExpander
Given the files found by search, expand to include architecturally related files:
1. **Import chain**: Files imported by search results (1 level deep)
2. **Reverse imports**: Files that import the search results
3. **Test files**: `test_*.py`, `*.test.ts`, `*_test.go` matching result files
4. **Config proximity**: If result is in `src/auth/`, include `src/auth/mod.rs` or `src/auth/index.ts`
5. **Tiered scoring (v2.2 fix):**
   - Direct imports and test files: **0.6x** multiplier (these are often critical context)
   - Reverse imports and sibling files: **0.4x** multiplier
   - Previous 0.3x was too aggressive — expanded files were always filtered out by the
     0.5 min_relevance threshold, making `include_related` effectively non-functional
6. **Expanded files are exempt from min_relevance filtering** (v2.2 fix) — they have already
   been validated as architecturally related. Token budget allocation handles prioritization.
7. **Chunk-level expansion (v2.3 fix):** When a related file is identified, do NOT include
   the entire file. Instead, run a quick vector similarity check against the query embedding
   on the file's chunks (already in LanceDB), and include only chunks scoring above 0.3.
   This prevents a 500-line middleware file from consuming token budget when only 20 lines
   are relevant. Previous file-level expansion was the #1 source of budget waste.
8. **Expansion caps (v2.3):** Max 5 expanded files per query, max 3 chunks per expanded file.
   Bounds the worst case for hub files that import/export dozens of modules.
9. **Hub-file detection (v2.3):** Skip files with >20 inbound or outbound imports during
   expansion (these are re-export barrels like `index.ts` or `mod.rs`, not meaningful deps).

#### QueryCache
LRU cache keyed on `(query_text, search_mode, project_hash)`:
- Cache search results (pre-assembly) for 60 seconds
- Each cache entry stores `files_included: HashSet<String>` alongside results
- **File-level invalidation (v2.2 fix):** On file change, only invalidate cached queries
  whose `files_included` set contains the changed file — NOT all cached queries for the project.
  This dramatically improves cache hit rates during active development.
- `bypass_cache` parameter available for forced fresh search
- Cache size: configurable, default 100 entries

### 5.4 Token Budget Allocation Strategy

When assembling context for a token budget:

```
1. Run hybrid search → get top-K results (K = top_k * 2, over-fetch for headroom)
2. Filter by min_relevance threshold (search results only)
3. Deduplicate overlapping chunks (merge only if >50% overlap)
4. If include_related: expand to related files (0.6x/0.4x tiered scoring)
   — expanded files are NOT filtered by min_relevance (v2.2)
5. Apply recency multiplier: final_score = rrf_score * recency_weight (v2.2)
6. Apply feedback adjustment: final_score *= feedback_multiplier (v2.2, if available)
7. Sort all chunks by final_score descending
8. Greedily allocate:
   for chunk in sorted_chunks:
       tokens = estimate_tokens(chunk.content)
       if running_total + tokens <= budget:
           include chunk
           running_total += tokens
       else:
           try truncating chunk to fit remaining budget
           break if even truncated chunk won't fit
9. Format output as ContextPackage
```

This greedy approach is simple, fast (<5ms), and produces good results. A more sophisticated
allocation (knapsack optimization) is possible in Phase 3 but unnecessary for v1.

### 5.5 Recency-Weighted Ranking

Recently modified files are more likely to be relevant to active development tasks.

- `last_modified` timestamp (Unix epoch) stored per chunk in LanceDB as an additional column
- Recency multiplier applied after RRF scoring — acts as **tiebreaker, not ranking override** (v2.2 fix):
  - Modified in last 24h: **1.1x** boost (was 1.3x — reduced to prevent irrelevant recent files from polluting top-5)
  - Modified in last 7d: **1.03x** boost (was 1.1x)
  - Older: **1.0x** (no change)
- **Minimum score gate (v2.2):** Recency boost only applies when `base_rrf_score > 0.3`.
  This prevents recently-edited but completely irrelevant files from being boosted into results.
- Formula: `final_score = rrf_score * (if rrf_score > 0.3 { recency_multiplier } else { 1.0 })`
- When git blame data is available (Phase 2), use last commit timestamp instead of file mtime for more accurate per-chunk recency
- Inspired by Augment's Context Lineage, which tracks edit event sequences per developer

### 5.6 Multi-Language Context Stitching

Full-stack queries (e.g., "how does user signup work") need context across languages.

- **Adaptive threshold (v2.2 fix):** The language diversity trigger is tied to project archetype:
  - **Monorepo / Full-stack projects**: trigger if >70% from one language (cross-language context expected)
  - **Single-language projects**: trigger if >95% from one language (most projects — avoids promoting Makefiles/Dockerfiles over relevant code)
  - Default (no archetype detected): 90% threshold
- When triggered, boost underrepresented language chunks by 1.2x and re-sort
- Cross-language relationship detection heuristics:
  - API route in Python/Go → TypeScript/JavaScript fetch call to that route
  - GraphQL schema definition → resolver implementation → frontend query
  - Database migration → ORM model → API handler
- Project archetype detection (heuristic from file structure):
  - **Monorepo**: multiple `package.json` / `Cargo.toml` at different depths
  - **Full-stack**: `src/` with both backend and frontend directories
  - **Microservices**: multiple independent service directories with own configs
- Archetype informs the language diversity threshold and related-file expansion strategy

### 5.7 Progressive Context Delivery

For large token budgets (>16K tokens), return context in layers to help agents decide if they need more detail:

- **Layer 1 (immediate)**: Top-5 highest-relevance chunks + file-level summaries for remaining files
  - File summary: `{file_path} ({language}, {line_count} lines) — contains: {symbol_list}`
  - Total tokens: typically 2-4K regardless of budget
- **Layer 2 (on request)**: Full chunks for all files within budget
  - Agent calls `get_context_for_prompt` again with `expand_files: ["src/auth/jwt.py"]`
- `include_summary` parameter (default: false) in `get_context_for_prompt` enables Layer 1 mode
- Reduces token consumption by 40-60% when agents only need an overview

### 5.8 Context Quality Feedback Loop

Closed-loop learning from agent feedback to improve future context assembly.

- `report_context_quality` MCP tool (see Section 8) accepts per-chunk usefulness signals
- **Feedback keying (v2.2.1 refined):**
  - AST chunks (with symbol_name): keyed on `(rel_path, symbol_name)` — survives re-indexing
  - Sliding-window chunks (no symbol_name): keyed on `(rel_path, start_line_bucket)` where
    `start_line_bucket = start_line / chunk_size` — stable across minor line shifts, distinguishes
    different sections of the same file (prevents contradictory signals from canceling out)
- Feedback stored in SQLite: `feedback (rel_path TEXT, chunk_key TEXT, useful INTEGER, timestamp INTEGER)`
  where `chunk_key` is either `symbol_name` or `line_bucket_{N}`
- **After 10+ signals (v2.2 fix, was 5)**, adjust ranking weight:
  - >70% marked useful: boost by **1.15x** in future queries
  - <30% marked useful: penalize by **0.87x**
  - Between 30-70%: no adjustment (inconclusive)
  - Symmetric multipliers (v2.2 fix): prevents gradual suppression from mixed feedback
- **30-day exponential decay (v2.2 fix):** Feedback signals lose weight over time with a 30-day
  half-life. This prevents stale negative signals from permanently suppressing refactored code.
  Alternatively, feedback is auto-invalidated when a chunk's content hash changes.
- Phase 3 enhancement: use aggregated feedback to fine-tune relevance thresholds per project
- Feedback is local-only — never leaves the developer's machine

---

## 6. Embedding Backend Strategy

### 6.1 Backend Comparison

| Backend | Model | Dims | Accuracy | Latency | Size | License |
|---|---|---|---|---|---|---|
| **Local ONNX (DEFAULT)** | **Jina Embeddings v2 Base Code** | **768** | **★★★★☆** | **~200-400ms/batch32** | **~300MB** | **Apache 2.0** |
| Local ONNX (lite) | BAAI/bge-small-en-v1.5 | 384 | ★★★☆☆ | ~50-100ms/batch32 | ~130MB | MIT |
| Cloud API | Codestral Embed (Mistral) | 3072 | ★★★★★ | ~50ms/batch | API | Proprietary |
| Cloud API | Voyage Code 3 | 2048 | ★★★★★ | ~60ms/batch | API | Proprietary |
| Cloud API | text-embedding-3-large (OpenAI) | 3072 | ★★★★☆ | ~90ms/batch | API | Proprietary |
| Ollama | nomic-embed-text | 768 | ★★★☆☆ | ~30ms/batch | ~270MB | Apache 2.0 |

**v2.1 change: Default model is now `jina-embeddings-v2-base-code`.** This is a code-specific
embedding model that provides 20-50% better retrieval accuracy on CodeSearchNet benchmarks
compared to general-purpose models like `bge-small-en-v1.5`.

**Why Jina v2 Base Code:**
- 137M parameters, 768 dimensions — good balance of quality vs speed
- Trained specifically on code: understands function signatures, variable names, docstrings
- **v2.3:** `OnnxEmbedder` is built directly against `ort` + `tokenizers` (primary approach).
  `fastembed-rs` is optional — its Rust crate may not support Jina v2 at the required version.
  ONNX model + tokenizer.json downloaded from HuggingFace on first run to `~/.vektor/models/`.
- Apache 2.0 license — fully open source, no usage restrictions
- 8K token context window — handles large functions without truncation

**Fallback:** Users on constrained machines (low RAM, no GPU) can use `--lite` flag to fall back
to `bge-small-en-v1.5` (130MB, 384d). Progress bar shown during initial model download.
Auto-select `--lite` if system RAM is below 12GB (detected at startup).

**Platform-agnostic hardware acceleration (v2.2):**
`ort` supports multiple execution providers auto-detected at runtime — no platform-specific code:
- **CPU (all platforms)**: Default. Uses SIMD (AVX2/NEON) automatically.
- **CUDA/TensorRT (NVIDIA GPU)**: Auto-detected if CUDA runtime is present. 5-10x speedup.
- **CoreML (macOS)**: Auto-detected on Apple Silicon. 2-3x speedup. Neural Engine utilization.
- **DirectML (Windows)**: Auto-detected on Windows with GPU. Works with AMD/Intel/NVIDIA.
- **OpenVINO (Intel)**: Auto-detected on Intel hardware. Optimized for Intel CPUs and iGPUs.

`ort` probes available providers at startup and selects the best one. No configuration needed.
The `load-dynamic` Cargo feature enables this. Vektor logs which provider was selected at startup.

**Realistic local inference latency (v2.2 correction):**
Previous PRD claimed "~30ms/batch" for local ONNX — this was cloud API latency, not local inference.
Actual local ONNX on Jina v2 (137M params): ~200-400ms per batch of 32 texts on modern CPU.
With hardware acceleration (GPU/CoreML/DirectML): ~50-100ms per batch of 32.
These numbers are used in updated performance targets.

**Cloud API rate limiting (v2.2):**
When using OpenAI-compatible embedding backends, a configurable `max_requests_per_minute`
(default: 500) with token bucket rate limiter prevents API spam during large initial indexes.
Without this, low-tier API keys get rate-limited on every batch.

**Cloud options (v2.1):**
- **Codestral Embed** (Mistral): Best code retrieval benchmarks, Matryoshka dimensions (reduce to 768d for storage savings)
- **Voyage Code 3**: Runner-up accuracy, 2048d, Matryoshka support
- Both accessible via OpenAI-compatible API format (`VEKTOR_OPENAI_BASE_URL` override)

### 6.2 Accuracy Enhancement Techniques

These techniques improve retrieval quality beyond just model selection:

1. **Symbol-enriched chunks** — Prepend function/class name + docstring to chunk content
   before embedding. Dramatically improves semantic alignment between query and result.

2. **Dual embedding** (Phase 3) — Embed both the code body AND a natural language description
   extracted from docstrings. Store both vectors as separate columns in LanceDB table.

3. **Query expansion** (Phase 3, optional) — Call a local LLM to expand the search query
   with synonyms before embedding. e.g., "auth" → "authentication, login, token, session, JWT".

4. **Cross-encoder re-ranking** (Phase 2, moved from Phase 3 in v2.3) — After RRF fusion,
   re-score top-20 results with a local cross-encoder model. Return top-K. This is the single
   highest-impact accuracy improvement: 10-15% Precision@5 lift at ~200ms additional latency.

5. **Chunk overlap** — 25% line overlap between adjacent chunks prevents semantic content
   from being split at chunk boundaries.

6. **Query prefix strategy** (v2.1) — Jina v2 Code model uses task-specific prefixes to
   improve retrieval accuracy by 5-10% on CodeSearchNet benchmarks:
   - When embedding document/code chunks: prefix with `"search_document: "`
   - When embedding search queries: prefix with `"search_query: "`
   - Prefix is added automatically by the `Embedder` trait — callers don't need to know about it
   - Other models that don't use prefixes simply skip this step

### 6.3 Configuration

```toml
# ~/.vektor/config.toml
# Config precedence: CLI args > env vars (VEKTOR_*) > this file > defaults

[embedding]
backend = "onnx"                   # onnx | openai | ollama
openai_api_key = ""               # OpenAI-compatible API key
openai_base_url = "https://api.openai.com/v1"  # Override for compatible APIs
openai_model = "text-embedding-3-small"
ollama_url = "http://localhost:11434"
ollama_model = "nomic-embed-text"
onnx_model = "jinaai/jina-embeddings-v2-base-code"  # code-optimized default (v2.1)
fallback_to_onnx = true           # auto-fallback if API unreachable
max_requests_per_minute = 500     # rate limit for cloud APIs (v2.2)

[index]
data_dir = "~/.vektor"
max_file_size_kb = 512
chunk_max_lines = 200             # max AST chunk size before sub-chunking (v2.2, was 80)
chunk_overlap_pct = 25            # for code files; docs use 40% automatically
doc_chunk_max_lines = 40          # smaller windows for *.md, *.txt, *.rst (v2.2)

[watcher]
debounce_ms = 200
enabled = true

[server]
mode = "stdio"                    # stdio | sse
```

---

## 7. Agent DX & Adoption Strategy

> **The best tool is one agents discover and use without human configuration.**
> Zero-friction adoption is as important as technical capabilities. If agents can't
> find Vektor, Vektor's features don't matter.

### 7.1 Design Philosophy: Zero-Friction Adoption

Goal: `cargo install vektor` → working context engine in <2 minutes, no configuration.

```
Install:    cargo install vektor           (or curl installer)
First use:  Agent calls any Vektor tool    (auto-detected via Skills or MCP config)
Auto-index: Vektor detects unindexed project → shallow index in <5s
Result:     Agent gets keyword search results immediately
Background: Full semantic index builds silently
Full power: Within 1-5 minutes, hybrid search + context assembly fully operational
```

Progressive enhancement is the key principle: every state of indexing returns something useful.

### 7.2 Skills Integration (SKILL.md)

Skills is an emerging open standard for teaching agents about available tools without
requiring MCP server configuration. Vektor ships a Skills file for zero-config discovery.

**How it works:**
- Ship `.claude/skills/vektor/SKILL.md` in user's project (or global `~/.claude/skills/`)
- Agents auto-discover skill files and learn when/how to use Vektor tools
- Compatible with: Claude Code, Cursor, Codex CLI, Windsurf (via agentskills.io standard)
- Installation: `npx skills add vektor` (for npm-based agents)

**Skill content describes:**
- All 8 MCP tools with usage guidance and example queries
- When to use `get_context_for_prompt` vs `search_code`
- How to interpret `index_status` in responses
- Recommended `token_budget` values for different agent contexts

### 7.3 First-Run Experience

On first tool call (any tool), Vektor detects an unindexed project and responds gracefully:

```
1. Agent calls get_context_for_prompt("how does auth work", budget=8000)
2. Vektor detects: no index exists for this project path
3. Immediate response (0ms): { results: [], index_status: "building", message: "Starting index..." }
4. Background: Shallow index begins (file walk + Tantivy BM25 from paths + first 50 lines)
5. Within 5s: Shallow index complete
6. Next agent call: returns keyword results with index_status: "partial"
7. Background: Deep index continues (tree-sitter chunking + embedding)
8. Within 1-5min: Full index complete
9. Subsequent calls: returns full hybrid results with index_status: "full"
```

**Never fails due to missing index.** Always degrades gracefully.

### 7.4 MCP Server Instructions

The `server_instructions` field in the MCP capabilities response is critical for agent discovery.
Claude Code defers MCP tools when >10% of context is consumed — keyword-rich descriptions
ensure Vektor tools are found by Claude Code's Tool Search (BM25 on tool descriptions).

**Server instructions content:**
```
Vektor is a local-first codebase context engine. Use get_context_for_prompt to get
token-budgeted, deduplicated code context for any query. Use search_code for hybrid
semantic + keyword code search. Use index_codebase to trigger indexing. All operations
are local — no code leaves the developer's machine.
```

### 7.5 MCP Resources

Vektor exposes MCP Resources for passive context that agents can subscribe to:

- **`vektor://project/status`** — index health, coverage percentage, active watchers
  - Agents can @-mention in Claude Code: `@vektor://project/status`
  - Updated when index state changes (building → partial → full)
- **`vektor://project/summary`** — codebase overview: languages, file count, top symbols
  - Useful for agents bootstrapping understanding of a new project
  - Updated after each full index completion

Send `notifications/resources/updated` when index completes so subscribed agents refresh context.

### 7.6 Distribution Strategy

| Channel | Command | Target |
|---|---|---|
| Cargo | `cargo install vektor` | Rust developers |
| GitHub Releases | Prebuilt binaries (macOS arm64/x86_64, Linux x86_64/arm64) | All developers |
| Installer script | `curl -fsSL https://install.vektor.dev \| sh` | Quick setup |
| Skills | `npx skills add vektor` | Agent-first users |
| Docker | `docker run vektor serve` | CI/testing environments |

**MCP config (Claude Code / Claude Desktop):**
```bash
claude mcp add --transport stdio vektor -- vektor serve
```

---

## 8. MCP Tools API

Vektor exposes 8 MCP tools callable by Claude Code, Codex CLI, Cursor, and any MCP client.

> **v2.1 change:** All tool responses now include `index_status` and `index_coverage_pct`
> fields so agents know the quality of results they're receiving (see Section 4.4).

### Primary Tools (used by agents in every session)

#### `get_context_for_prompt` — THE KILLER FEATURE
Assembles a token-budgeted, deduplicated, relationship-aware context package optimized for
LLM consumption. This is what makes agents 30-80% more effective.

```json
{
  "path": "/absolute/path/to/project",
  "query": "how does authentication and session management work",
  "token_budget": 8000,
  "max_files": 10,
  "include_related": true,
  "min_relevance": 0.5,
  "include_docs": true,
  "bypass_cache": false,
  "scope": "packages/auth-service"
}
```

Response:
```json
{
  "context": [
    {
      "file": "src/auth/jwt.py",
      "lines": "1-45",
      "symbol": "validate_token",
      "type": "function_definition",
      "language": "python",
      "relevance": 0.891,
      "source": "search",
      "content": "def validate_token(token: str) -> Claims:\n    ..."
    },
    {
      "file": "src/auth/middleware.py",
      "lines": "10-38",
      "symbol": "AuthMiddleware",
      "type": "class_definition",
      "language": "python",
      "relevance": 0.72,
      "source": "related",
      "content": "class AuthMiddleware:\n    ..."
    },
    {
      "file": "tests/test_auth.py",
      "lines": "1-30",
      "symbol": "test_validate_token",
      "type": "function_definition",
      "language": "python",
      "relevance": 0.45,
      "source": "related",
      "content": "def test_validate_token():\n    ..."
    }
  ],
  "metadata": {
    "files_included": 5,
    "total_tokens": 6420,
    "budget_used_pct": 80.2,
    "chunks_returned": 8,
    "chunks_deduplicated": 3,
    "search_time_ms": 45,
    "cache_hit": false,
    "index_status": "full",
    "index_coverage_pct": 99.8,
    "result_confidence": "high",
    "budget_gap_reason": "no_more_relevant",
    "suggested_action": null,
    "clusters": [
      {"path": "src/auth/jwt", "chunk_count": 3, "avg_relevance": 0.85},
      {"path": "src/auth/middleware", "chunk_count": 2, "avg_relevance": 0.72}
    ]
  }
}
```

**`scope` parameter (v2.3):** Optional workspace-relative path to bias results toward a
specific package/directory. Critical for monorepos where agents work in one package at a time.
When set, results from the scoped path get a 1.5x score boost, and related expansion is
limited to the scope and its direct dependencies. Without scoping, a 200K-file monorepo
produces noisy cross-package results. Workspace roots are auto-detected (Cargo workspaces,
npm workspaces, Go modules).

**Confidence signaling (v2.3):** The `result_confidence`, `budget_gap_reason`, and
`suggested_action` fields enable agents to make informed decisions:
- `"result_confidence": "low"` + `"suggested_action": "try broader query"` → agent reformulates
- `"result_confidence": "medium"` + `"budget_gap_reason": "index_incomplete"` → agent waits for full index
- `"clusters"` field appears when results span >2 distinct code areas, enabling disambiguation

#### `search_code`
Hybrid BM25 + semantic vector search. Returns raw ranked results (for agents that want
to assemble context themselves).

```json
{
  "path": "/absolute/path/to/project",
  "query": "function that handles JWT token validation",
  "top_k": 8,
  "mode": "hybrid",
  "filter_ext": [".py"],
  "bypass_cache": false
}
```

`mode` options: `hybrid` (default), `semantic` (vector only), `keyword` (BM25 only)

Response:
```json
{
  "results": [
    {
      "file": "src/auth/jwt.py",
      "lines": "42-67",
      "symbol": "validate_token",
      "type": "function_definition",
      "language": "python",
      "score": 0.891,
      "snippet": "def validate_token(token: str) -> Claims:\n    ..."
    }
  ],
  "metadata": {
    "search_time_ms": 32,
    "mode": "hybrid",
    "cache_hit": false,
    "index_status": "full",
    "index_coverage_pct": 99.8
  }
}
```

### Indexing Tools

#### `index_codebase`
Index or incrementally update a codebase. Uses SHA-256 file hash diffing — only changed
files are re-processed. Also indexes documentation files (*.md, *.txt, *.rst).

```json
{
  "path": "/absolute/path/to/project",
  "force_full": false,
  "extensions": [".py", ".ts", ".md"],
  "embedding_backend": "onnx"
}
```

#### `get_index_status`
Returns indexing stats, health, and coverage.

```json
{ "path": "/absolute/path/to/project" }
```

Response:
```json
{
  "indexed": true,
  "files_total": 3240,
  "files_indexed": 3238,
  "files_failed": 2,
  "chunks_total": 18420,
  "last_indexed_at": "2026-03-01T10:30:00Z",
  "embedding_backend": "onnx",
  "embedding_dim": 768,
  "index_size_mb": 145,
  "watcher_active": true
}
```

#### `clear_index`
Wipe index for a project. Next `index_codebase` will do a full re-index.

```json
{ "path": "/absolute/path/to/project" }
```

### Discovery Tools (Phase 2)

#### `find_symbol`
Locate where a specific function, class, or variable is defined.

```json
{
  "path": "/absolute/path/to/project",
  "symbol_name": "validate_token",
  "symbol_type": "function"
}
```

#### `find_dependencies`
Returns dependency graph: what a file imports OR what files import this file.

```json
{
  "path": "/absolute/path/to/project",
  "file": "src/auth/jwt.py",
  "direction": "imported_by"
}
```

### Feedback Tools (Phase 2)

#### `report_context_quality`
Accepts agent feedback on which context chunks were useful vs irrelevant. Enables
closed-loop learning for improved ranking over time (see Section 5.8).

```json
{
  "path": "/absolute/path/to/project",
  "query_id": "abc123",
  "useful_chunks": ["src/auth/jwt.py:1-45", "src/auth/middleware.py:10-38"],
  "irrelevant_chunks": ["src/utils/logging.py:10-30"],
  "feedback": "The auth middleware was more relevant than the JWT implementation"
}
```

Response:
```json
{
  "status": "recorded",
  "feedback_count": 42,
  "message": "Feedback stored. Ranking adjustments will apply after 10+ signals per chunk."
}
```

### MCP Resources

Vektor exposes MCP Resources for passive context subscription (see Section 7.5):

| Resource URI | Description | Updates |
|---|---|---|
| `vektor://project/status` | Index health, coverage %, watcher status | On index state change |
| `vektor://project/summary` | Languages, file count, top symbols, architecture type | After full index |

### MCP Config (Claude Code / Claude Desktop)

```json
{
  "mcpServers": {
    "vektor": {
      "command": "/usr/local/bin/vektor",
      "args": ["serve"],
      "env": {
        "VEKTOR_EMBED_BACKEND": "onnx",
        "VEKTOR_OPENAI_API_KEY": "",
        "VEKTOR_DATA_DIR": "~/.vektor"
      }
    }
  }
}
```

> **Env var override convention:** Any config key can be overridden via `VEKTOR_` prefix.
> e.g., `VEKTOR_EMBED_BACKEND=openai`, `VEKTOR_OPENAI_BASE_URL=https://api.voyageai.com/v1`

---

## 9. Performance Targets

| Metric | Target | Stretch Goal | Comparison |
|---|---|---|---|
| Full index: 10K files, `--lite` (bge-small 384d, CPU) | **<180s** | <90s | Recommended for fast indexing |
| Full index: 10K files, Jina v2 768d (CPU) | **<600s** | <300s | Higher accuracy, ~3x slower (v2.3 revised) |
| Full index: 10K files (GPU/CoreML accel) | <60s | <30s | With CUDA/CoreML/DirectML |
| Full index: 100K file codebase (CPU, `--lite`) | <20min | <10min | Augment: ~15min (cloud) |
| Incremental re-index (1 file change) | <500ms | <200ms | Augment: ~300ms |
| `search_code` (local ONNX embed) | <150ms (P95) | <100ms (P50) | Zilliz: ~400ms (cloud RTT) (v2.3 revised) |
| `search_code` (cloud API embed query) | <300ms | <150ms | Zilliz: ~600ms |
| **`get_context_for_prompt`** (local ONNX) | **<200ms** | **<100ms** | Augment: ~200ms (cloud) (v2.3 revised) |
| **Context assembly overhead** | **<10ms** | **<5ms** | (dedup + budget + format) |
| **Cache hit response** | **<5ms** | **<2ms** | — |
| Binary size (without ONNX runtime) | <50MB | <30MB | — |
| RAM: 10K files indexed (Jina v2) | **<700MB** | <500MB | ONNX model ~300MB + index (v2.3 revised) |
| RAM: 10K files indexed (`--lite`) | **<400MB** | <250MB | bge-small model ~130MB (v2.3) |
| RAM: 100K files indexed | <2GB | <1.2GB | Augment: ~3GB |
| Precision@5 code retrieval (Phase 1) | **>0.65** | >0.72 | Zilliz baseline: ~0.65 (v2.3 revised) |
| Precision@5 code retrieval (Phase 2, with re-ranker) | **>0.78** | >0.85 | CocoIndex: ~0.55-0.60 (v2.3) |
| **Shallow index (keyword-ready)** | **<5s** | **<2s** | **CocoIndex: no equivalent** |
| **Time to first search result (new project)** | **<8s** | **<3s** | **Augment: ~60s** |
| **Skills discovery by agent** | **<1s** | **—** | **CocoIndex: ~1s (npx skills)** |
| **Feedback ingestion** | **<10ms** | **<5ms** | **—** |
| **ONNX warm-up (first query)** | **<5s at startup** | **<2s** | **Without warm-up: 3-5s cold start** |
| **Watcher re-index with chunk cache** | **<300ms** | **<150ms** | **Without cache: ~350ms** |

> **v2.3 performance note:** CPU targets for full indexing with Jina v2 768d were revised
> upward after analysis showed embedding is the bottleneck (~200-400ms per batch of 32 on CPU).
> The `--lite` flag (bge-small, 384d, ~50-100ms/batch) is the recommended fast path.
> GPU/CoreML acceleration brings Jina v2 to the original targets.

---

## 10. Technology Stack

| Component | Crate | Why |
|---|---|---|
| MCP Protocol | `rmcp` v0.16+ | Official Rust MCP SDK, stdio + SSE transport |
| File Watching | `notify` v6 | OS-native events, cross-platform |
| AST Chunking | `tree-sitter` + language grammars | Official Rust crate, 40+ languages |
| ONNX Inference | `ort` v2 + `tokenizers` | ONNX Runtime (primary) + HuggingFace tokenizer for model input (v2.3: `ort` is primary, `fastembed` optional) |
| Tokenization | `tokenizers` | HuggingFace tokenizer for ONNX models |
| Vector Database | `lancedb` (embedded) | True in-process embedded, Arrow-based, no server |
| Full-Text Search | `tantivy` | Pure Rust BM25, Lucene equivalent |
| Parallelism | `rayon` + `tokio` | CPU work + async I/O |
| File Hashing | `sha2` | Fast SHA-256 for incremental diffing |
| gitignore parsing | `ignore` | Same crate as ripgrep, battle-tested |
| HTTP Client | `reqwest` | OpenAI / Ollama embedding API calls |
| Serialization | `serde` + `serde_json` | MCP JSON-RPC protocol |
| Arrow IPC | `arrow`, `arrow-array`, `arrow-schema` | LanceDB schema and data interchange |
| Config | `config` + `toml` | TOML file + env var override |
| State storage | `rusqlite` | Hash store, dependency graph |
| CLI | `clap` | Argument parsing for binary |
| Async traits | `async-trait` | Async methods in trait definitions |
| Path resolution | `dirs` | Cross-platform home/config directory resolution |
| Progress display | `indicatif` | Progress bars for indexing operations |
| Caching | `lru` | LRU cache for query result caching |
| Git integration | `git2` | libgit2 bindings for commit history indexing (Phase 2) |
| Error handling | `anyhow` + `thiserror` | Ergonomic Rust error handling |
| Logging | `tracing` + `tracing-subscriber` | Structured async-aware logging |
| System info | `sysinfo` | Cross-platform RAM detection for auto-selecting `--lite` model |

### Cargo.toml (skeleton)

```toml
[package]
name = "vektor"
version = "0.1.0"
edition = "2021"
description = "Local-first codebase indexing MCP server"
license = "MIT"

[dependencies]
# MCP Protocol
rmcp = { version = "0.16", features = ["server", "transport-io"] }
tokio = { version = "1", features = ["full"] }

# File system
notify = "6"
ignore = "0.4"
walkdir = "2"

# AST Parsing
tree-sitter = "0.24"
tree-sitter-python = "0.23"
tree-sitter-typescript = "0.23"
tree-sitter-javascript = "0.23"
tree-sitter-rust = "0.23"
tree-sitter-go = "0.23"

# Embedding (v2.3: ort is primary, fastembed optional)
ort = "2"                          # ONNX Runtime — primary embedding backend (static CPU link by default)
tokenizers = "0.20"                # HuggingFace tokenizer for ONNX model input construction
tiktoken-rs = "0.5"                # Precise token counting for two-pass budget verification (v2.3)
reqwest = { version = "0.12", features = ["json"] }
# fastembed = "4"                  # Optional convenience layer — uncomment if fastembed-rs supports Jina v2

# Vector Storage (LanceDB embedded)
# NOTE: Do NOT independently pin arrow versions. Let lancedb dictate the arrow version
# via its transitive dependency to avoid type-mismatch compilation failures. (v2.3 fix)
lancedb = "0.23"
# arrow, arrow-array, arrow-schema versions are inherited from lancedb

# Full-Text Search
tantivy = "0.22"

# State Storage
rusqlite = { version = "0.31", features = ["bundled"] }

# Utilities
rayon = "1"
sha2 = "0.10"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
thiserror = "2"
async-trait = "0.1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
clap = { version = "4", features = ["derive"] }
config = "0.14"
toml = "0.8"
dirs = "5"
indicatif = "0.17"
uuid = { version = "1", features = ["v4"] }
lru = "0.12"
sysinfo = "0.32"

# Phase 2 (uncomment when needed)
# git2 = "0.19"
# regex = "1"
```

---

## 11. Phased Implementation Plan

> **Core principle: One function at a time. Understand every line before moving to the next.**
> Each function below is a discrete unit of work. Implement, test, understand, then proceed.

---

### Phase 1 — Core Engine (Weeks 1–6)

Goal: Working MCP server that indexes a codebase and serves hybrid search results
with local ONNX embeddings. No real-time watching yet.

> **Timeline note:** 6 weeks is realistic for production-grade code with thorough testing.
> The first 2 weeks build foundational Rust skills. Weeks 3-4 tackle the complex embedding
> and storage layer. Weeks 5-6 integrate everything into a working MCP server.

#### CLI Modes
Vektor operates in two modes:
- **One-shot CLI**: `vektor index /path/to/project` — index and exit. For testing and CI.
- **Server mode**: `vektor serve` — start MCP server (+ optional file watcher), stay running.
  This is the production mode used by Claude Code / Codex CLI.

---

#### Week 1 — Project Skeleton + File Discovery

**Function 1.1: `main()`**
- Set up Rust binary with `clap` for CLI args
- Initialize `tracing` for logging
- Parse config from TOML file + env vars
- Start tokio async runtime
- What you learn: Rust binary entry points, async runtime setup, error handling with `anyhow`

**Function 1.2: `discover_files(root: &Path, config: &Config) -> Vec<PathBuf>`**
- Use the `ignore` crate to walk directory tree
- Automatically respect `.gitignore`, `.git/`, `node_modules/`, `__pycache__/`, etc.
- Filter by supported file extensions
- Skip files larger than `max_file_size_kb`
- Return sorted list of `PathBuf`
- What you learn: `ignore` crate API, `PathBuf` vs `&Path`, iterator chaining in Rust

**Function 1.3: `hash_file(path: &Path) -> Result<String>`**
- Read file bytes
- SHA-256 hash via `sha2` crate
- Return hex string of first 16 chars (sufficient for change detection)
- What you learn: File I/O in Rust, `sha2` crate, hex encoding

**Function 1.4: `HashStore` struct + `get_hash()` + `set_hash()` + `is_changed()`**
- SQLite via `rusqlite`
- Table: `file_hashes (rel_path TEXT PRIMARY KEY, hash TEXT, status TEXT DEFAULT 'pending', indexed_at INTEGER)`
- Status values: `pending` (queued), `indexed` (successfully processed), `failed` (error during indexing)
- `get_hash(rel_path)` → `Option<String>`
- `set_hash(rel_path, hash, status)` → `Result<()>`
- `is_changed(path, root)` → `bool` (compares current hash vs stored)
- `get_pending()` → `Vec<String>` (files needing re-index after crash recovery)
- What you learn: `rusqlite` CRUD, SQLite schema design, `Option<T>` in Rust, crash recovery patterns

---

#### Week 2 — AST Chunking

**Function 2.1: `detect_language(path: &Path) -> Option<Language>`**
- Match file extension to tree-sitter Language enum
- Return `None` for unsupported extensions (will use fallback chunker)
- What you learn: Rust enums, pattern matching with `match`, `Option<T>`

**Function 2.2: `parse_ast(content: &str, language: Language) -> Result<Tree>`**
- Initialize tree-sitter `Parser` with the detected language grammar
- Parse content bytes into a `Tree`
- What you learn: tree-sitter Rust API, unsafe FFI basics (tree-sitter uses C under the hood)

**Function 2.3: `extract_chunks_ast(tree: &Tree, content: &str, language: Language) -> Vec<Chunk>`**
- Walk AST nodes
- Identify chunk-worthy node types per language:
  - Python: `function_definition`, `class_definition`
  - TypeScript/JS: `function_declaration`, `method_definition`, `class_declaration`
  - Rust: `function_item`, `impl_item`, `struct_item`
  - Go: `function_declaration`, `method_declaration`
- For each matching node: extract text, start_line, end_line, symbol_name
- Prepend symbol name + docstring/comment to content for richer embeddings
- What you learn: Tree traversal in Rust, recursive functions, string slicing

**Function 2.4: `extract_chunks_sliding(content: &str, max_lines: usize, overlap_pct: f32) -> Vec<Chunk>`**
- Fallback for unsupported languages
- Split into overlapping line windows
- 80 lines per chunk, 25% overlap = 20 line overlap
- What you learn: Rust slice operations, `.chunks()`, `.windows()` on iterators

**Function 2.5: `chunk_file(path: &Path, content: &str) -> Vec<Chunk>`**
- Top-level dispatcher: try AST chunking → fall back to sliding window
- Returns `Vec<Chunk>` regardless of which strategy succeeded
- What you learn: composing functions in Rust, Result chaining with `?`

**`Chunk` struct:**
```rust
pub struct Chunk {
    pub content: String,
    pub content_hash: String,        // SHA-256 of content — for embedding cache (v2.2)
    pub rel_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub symbol_name: Option<String>,
    pub symbol_type: Option<String>,
    pub language: String,
}
```

---

#### Week 3 — Embedding + Vector Storage

**Function 3.1: `Embedder` trait**
```rust
#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>>;
    fn dim(&self) -> usize;
    fn name(&self) -> &str;
    fn prefix_for_document(&self) -> &str { "" }  // "search_document: " for Jina v2 (v2.2)
    fn prefix_for_query(&self) -> &str { "" }      // "search_query: " for Jina v2 (v2.2)
}
```
- Each backend implements prefix methods; callers call `embed_documents()` or `embed_query()`
  helper methods that auto-prepend the correct prefix
- What you learn: Rust traits, `async_trait` macro, `Send + Sync` bounds, default trait methods

**Function 3.2: `OnnxEmbedder::new(model_name: &str) -> Result<Self>`** (v2.3: `ort`-direct approach)
- Load ONNX model via `ort` crate directly (NOT `fastembed`, which may lack Jina v2 support)
- Download ONNX model from HuggingFace on first run (stored in `~/.vektor/models/`)
- Load matching `tokenizer.json` for the model (via `tokenizers` crate)
- Configure ONNX session: `GraphOptimizationLevel::Level3`, static CPU provider by default
- **Warm-up:** Embed a dummy string at init for batch_size=1 and batch_size=32 to trigger
  ONNX JIT compilation. Consider padding to powers of 2 (1, 2, 4, 8, 16, 32) for all shapes.
- What you learn: `ort` API, model loading, HuggingFace model format, `tokenizers` crate

**Function 3.3: `OnnxEmbedder::embed(texts: Vec<String>) -> Result<Vec<Vec<f32>>>`**
- Prepend query/document prefix before tokenization (e.g., `"search_document: "` for Jina v2)
  — prefix must come before any content enrichment (function signature, docstring)
- Tokenize input texts (use `tokenizers` crate with model's `tokenizer.json`)
- Construct input tensors: `input_ids`, `attention_mask`, `token_type_ids` (model-dependent)
- Run ONNX inference session
- Extract embedding vectors from output tensor, apply mean pooling over token embeddings
- L2-normalize vectors (required for cosine similarity via dot product)
- Process in batches of 32 to avoid OOM
- What you learn: tensor operations, batch processing, normalization math, tokenizer API

**Function 3.4: `OpenAiCompatEmbedder::embed(texts: Vec<String>) -> Result<Vec<Vec<f32>>>`**
- HTTP POST to OpenAI-compatible `/v1/embeddings` endpoint via `reqwest`
- Configurable `base_url` supports OpenAI, Voyage AI, Cohere, any compatible API
- Parse JSON response into `Vec<Vec<f32>>`
- Retry with exponential backoff on rate limit (429) or transient errors
- What you learn: async HTTP in Rust, JSON deserialization, retry patterns

**Function 3.5: `build_embedder(config: &Config) -> Result<Box<dyn Embedder>>`**
- Factory function: read `config.embedding.backend`
- Try primary backend → fall back to ONNX if unreachable
- Return `Box<dyn Embedder>` (trait object)
- What you learn: trait objects, dynamic dispatch in Rust, factory pattern

**Function 3.6: `VectorStore::new(project_dir: &Path, dim: usize) -> Result<Self>`**
- Connect to LanceDB at `~/.vektor/{hash}/lance/`
- Create table with Arrow schema if it doesn't exist:
  ```
  LanceDB Arrow Schema:
    - id            Utf8        — content-addressed chunk ID (PRIMARY KEY for merge operations)
                                  AST: sha256(rel_path:symbol_name:content_hash)
                                  Sliding-window: sha256(rel_path:chunk_{index}:content_hash) (v2.2.1 fix)
    - content_hash  Utf8        — SHA-256 of chunk content (embedding cache key, v2.2)
    - vector        FixedSizeList[Float32, dim]  — embedding vector (ANN indexed)
    - rel_path      Utf8        — relative file path (filter predicate, delete key)
    - start_line    UInt32      — chunk start line (display only)
    - end_line      UInt32      — chunk end line (display only)
    - symbol_name   Utf8        — function/class name, nullable (feedback key)
    - symbol_type   Utf8        — "function_definition", "class_definition", etc.
    - language      Utf8        — "python", "typescript", etc. (filter predicate)
    - content       Utf8        — chunk text (returned in search results)
    - last_modified Int64       — file mtime Unix epoch (recency scoring, v2.2)

  Indexes:
    - ANN index on `vector` column (IVF_PQ, built after initial indexing)
    - Metadata filter on `rel_path` (used by delete_by_file, embedding cache queries)
    - Metadata filter on `language` (used by search filter predicates)
  ```
- Store model info in `project_metadata` table (see Section 4.10) for dimension migration (v2.2)
- What you learn: LanceDB Rust API, Arrow schema definition, async initialization

**Function 3.7: `VectorStore::reindex_file(rel_path: &str, chunks: &[Chunk], embedder: &dyn Embedder) -> Result<()>`** (v2.2.1)
- First query existing chunks by `rel_path` → `HashMap<content_hash, Vec<f32>>` (read-before-delete)
- Then call `delete_by_file(rel_path)` to remove all existing chunks
- For each new chunk: reuse embedding from HashMap if `content_hash` matches, else embed
- Build Arrow `RecordBatch` from chunk+vector pairs (mix of reused and new embeddings)
- Chunk ID: AST chunks use `sha256(rel_path:symbol_name:content_hash)`,
  sliding-window chunks use `sha256(rel_path:chunk_{index}:content_hash)` (v2.2.1 fix)
- Insert new records (no merge_insert needed since we delete first)
- Process in batches of 500 records
- What you learn: Arrow RecordBatch construction, delete-then-insert pattern, batch operations

**Function 3.8: `VectorStore::search(query_vec: Vec<f32>, top_k: usize, filter: Option<&str>) -> Result<Vec<SearchResult>>`**
- ANN search via `table.query().nearest_to(&query_vec)`
- Optional SQL-like filter predicate (e.g., `language = 'python'`, `rel_path LIKE 'src/%'`)
- Return top-k results with score + all metadata columns
- What you learn: LanceDB query API, Arrow data extraction, optional parameters in Rust

**Function 3.9: `VectorStore::delete_by_file(rel_path: &str) -> Result<()>`**
- Delete all chunks for a given file path (needed for re-indexing)
- Uses LanceDB delete with predicate: `rel_path = '{rel_path}'`
- What you learn: LanceDB delete API, data lifecycle management

---

#### Week 4 — BM25 + Hybrid Search + MCP Server

**Function 4.1: `TextIndex::new(project_dir: &Path) -> Result<Self>`**
- Initialize Tantivy index on disk
- Schema matches formal definition in Section 4.10:
  `chunk_id` (Utf8, stored), `rel_path` (string + stored), `content` (text, tokenized with en_stem),
  `symbol_name` (text, tokenized + stored, 2.0x boost), `language` (string, stored),
  `start_line` (u64, stored), `end_line` (u64, stored), `index_depth` (string, stored)
- What you learn: Tantivy schema builder, field types, BM25 field boosting

**Function 4.2: `TextIndex::add_chunks(chunks: &[Chunk]) -> Result<()>`**
- Create Tantivy `Document` for each chunk
- Add to index writer
- Commit after all chunks added
- What you learn: Tantivy document API, index writer, commit semantics

**Function 4.3: `TextIndex::search(query: &str, top_k: usize) -> Result<Vec<SearchResult>>`**
- Parse query with Tantivy `QueryParser`
- Search against `content` and `symbol_name` fields
- Return top-k with BM25 score
- What you learn: Tantivy query parser, search API, scoring

**Function 4.4: `rrf_fuse(semantic: Vec<SearchResult>, keyword: Vec<SearchResult>, k: f32) -> Vec<SearchResult>`**
- Pure function — no I/O, easy to test
- Merge and score: `rrf_score = Σ 1 / (k + rank_i)`
- Apply weights: semantic 0.6, keyword 0.4
- Return sorted by combined score
- What you learn: HashMap usage in Rust, sorting with `sort_by`, floating point math

**Function 4.5: `search_hybrid(query: &str, config: &SearchConfig) -> Result<Vec<SearchResult>>`**
- Embed query with configured embedder
- Run `VectorStore::search` + `TextIndex::search` concurrently with `tokio::join!`
- Fuse results with `rrf_fuse`
- Truncate snippets to 400 chars
- Return final ranked list
- What you learn: `tokio::join!` for concurrent async tasks

**Function 4.6: `index_codebase(path: &Path, config: &Config, force: bool) -> Result<IndexStats>`**
- Orchestrator: calls 1.2 → 1.3 → 1.4 → 2.5 → 3.x → 4.x in order
- Parallel file processing with `rayon::par_iter()`
- Collect chunks from all files
- Embed in streaming batches of 500 chunks (avoids loading all vectors into memory at once)
- Track per-file status in HashStore: `pending` → `indexed` | `failed`
- On restart or crash recovery: resume from files still in `pending`/`failed` state
- Show progress bar via `indicatif` (files processed / total, chunks/sec, ETA)
- Return stats: files_indexed, skipped_unchanged, failed, total_chunks, elapsed_ms
- What you learn: `rayon` parallel iterators, orchestrating async + sync code, crash recovery

**Function 4.7: MCP server bootstrap**
- Initialize `rmcp` server
- Register 8 tools (get_context_for_prompt, search_code, index_codebase, get_index_status, find_symbol, find_dependencies, clear_index, report_context_quality)
- Handle tool dispatch in a `match` block
- Run stdio transport loop
- What you learn: MCP protocol, stdio JSON-RPC, async server loops in tokio

**Function 4.8: Each MCP tool handler**
- `handle_index_codebase(args)` — parse args, call `index_codebase()`, serialize response
- `handle_search_code(args)` — parse args, call `search_hybrid()`, serialize response
- `handle_get_context_for_prompt(args)` — parse args, call context assembly pipeline
- `handle_get_status(args)` — query `HashStore` and `VectorStore` for stats
- `handle_clear_index(args)` — drop LanceDB table + Tantivy index + hash store
- What you learn: JSON deserialization in Rust, error handling across async boundaries

---

#### Weeks 5–6 — Context Assembly Layer (THE DIFFERENTIATOR)

> This is what transforms Vektor from a search engine into a context engine.
> Without this, we're just a better grep. With this, we compete with Augment.

**Function CA.1: `TokenCounter::estimate(text: &str) -> usize`**
- Fast heuristic: `tokens ≈ bytes / 3.5` for code
- Empirically accurate within 5% for most programming languages
- Optional precise mode using `tiktoken-rs` for budget verification
- What you learn: performance vs accuracy trade-offs, byte-level string ops

**Function CA.2: `Deduplicator::deduplicate(chunks: Vec<SearchResult>) -> Vec<SearchResult>`**
- Sort by file path + start_line
- Merge overlapping line ranges from same file (keep higher-scoring chunk)
- Recalculate content from merged line range
- Pure function — no I/O, easy to unit test
- What you learn: interval merging algorithm, sorting with custom comparators

**Function CA.3: `RelatedExpander::expand(files: &[String], project: &ProjectState) -> Vec<ExpandedFile>`**
- Given files from search results, find architecturally related files:
  - Import chain (1 level): files imported by results
  - Reverse imports: files that import the results
  - Test files: pattern matching (`test_*.py`, `*.test.ts`, `*_test.go`)
  - Sibling files: `mod.rs`, `index.ts` in same directory
- Tiered scoring: 0.6x for imports/tests, 0.4x for siblings/reverse imports (v2.2 fix, was 0.3x)
- Expanded files exempt from min_relevance filtering (v2.2 fix)
- Uses DependencyGraph when available (Phase 2), falls back to file-system heuristics
- What you learn: graph traversal, pattern matching, heuristic scoring

**Function CA.4: `QueryCache::new(max_entries: usize) -> Self`**
- LRU cache using `lru` crate or custom `HashMap` + `VecDeque`
- Key: `(query_hash, search_mode, project_hash)`
- TTL: 60 seconds
- `get(key)` → `Option<Vec<SearchResult>>`
- `put(key, results)` → `()`
- `invalidate_project(project_hash)` — called by file watcher on change events
- What you learn: LRU cache implementation, cache invalidation strategies

**Function CA.5: `ContextAssembler::assemble(results: Vec<SearchResult>, config: &AssemblyConfig) -> Result<ContextPackage>`**
- The orchestrator that ties everything together:
  1. Check cache → return cached if hit
  2. Filter by `min_relevance`
  3. Deduplicate overlapping chunks
  4. If `include_related`: expand with `RelatedExpander`
  5. Sort all chunks by relevance descending
  6. Greedy token budget allocation (include chunks until budget exhausted)
  7. Format into `ContextPackage`
  8. Store in cache
- What you learn: pipeline composition, struct builders in Rust

**Function CA.6: `handle_get_context_for_prompt(args) -> Result<ContextPackage>`**
- MCP tool handler: parse args → search → assemble → respond
- This is the integration point: search_hybrid() → ContextAssembler::assemble()
- Returns the full `ContextPackage` JSON to the agent
- What you learn: end-to-end feature integration

**Function CA.7: `ShallowIndexer::build(root: &Path) -> Result<()>`** (v2.1)
- Walk directory tree using `ignore` crate (respects .gitignore)
- For each file: read first 50 lines + last 20 lines (captures exports/module.exports) + extract function/class declaration lines via regex
- Build Tantivy BM25 index from file paths + partial content
- Must complete in <5s for a 10K-file codebase
- This is the "Phase 1" of two-tier indexing — makes keyword search available immediately
- What you learn: streaming I/O, partial file reads, Tantivy batch insertion

**Function CA.8: `IndexStatusTracker`** (v2.1)
- Enum: `IndexPhase { None, Building, Partial, Full }`
- Tracks current indexing phase per project
- All MCP tool handlers query this before choosing search strategy:
  - `None` → trigger shallow index, return empty with status
  - `Partial` → use keyword-only search via Tantivy
  - `Full` → use hybrid search (Tantivy + LanceDB)
- Thread-safe via `Arc<RwLock<IndexPhase>>`
- What you learn: shared state in async Rust, RwLock, enum-driven control flow

**Function CA.9: `RecencyTracker::score(path: &str, base_score: f32) -> f32`** (v2.2 updated)
- Read file `mtime` from filesystem metadata
- Compute recency multiplier: 1.1x (24h), 1.03x (7d), 1.0x (older)
- **Minimum score gate:** only apply multiplier if `base_score > 0.3` (prevents boosting irrelevant files)
- Cached: re-read mtime only on file change events from watcher
- Integrates with ContextAssembler: `final_score = base_score * recency_multiplier`
- What you learn: filesystem metadata, time arithmetic in Rust

**Function CA.10: `SynonymExpander::expand(query: &str) -> Vec<String>`** (v2.2)
- Static synonym map (~50 entries) for common code concepts
- Returns expanded terms for BM25 query (OR across synonyms)
- Loaded once at startup from embedded HashMap — zero I/O during queries
- What you learn: HashMap in Rust, query expansion patterns

**Function CA.11: `AdaptiveWeights::compute(query: &str) -> (f32, f32)`** (v2.2.1 refined)
- Count identifier tokens (camelCase, snake_case, dot.notation) and compute density ratio
- Return (semantic_weight, keyword_weight) tuple based on density:
  - `>60%` identifiers: (0.4, 0.6) — favor BM25
  - `<25%` identifiers: (0.7, 0.3) — favor semantic
  - `25-60%` mixed: (0.6, 0.4) — balanced default
- Density-based, not binary — prevents "how does validate_token work" from getting BM25-heavy weights
- What you learn: regex patterns, density classification, adaptive algorithm design

**Function CA.12: `WarmUp::run(embedder: &dyn Embedder) -> Result<()>`** (v2.2.1 refined)
- Embed a single dummy string AND a batch of 32 dummy strings during `vektor serve` startup
- Pre-warms ONNX for both query path (batch_size=1) and indexing path (batch_size=32)
- Eliminates 3-5s cold start on first real query and ~500ms shape recompilation on first index batch
- Total warm-up cost: ~400-600ms at startup — acceptable
- What you learn: ONNX session lifecycle, shape-specific JIT compilation, startup optimization

**Skills file: `.claude/skills/vektor/SKILL.md`** (v2.1)
- Create Skills file describing all 8 MCP tools with usage guidance
- Include example queries and recommended parameter values
- Shipped as part of `vektor init` or `npx skills add vektor`

---

### Phase 2 — Real-Time + Git History + Accuracy (Weeks 7–12)

Goal: File-save triggers instant re-index. Ollama embedding backend.
Dependency graph. Git history indexing. Symbol search. Multi-project isolation.

**Function 5.1: `Watcher::new(root: &Path, tx: Sender<WatchEvent>) -> Result<Self>`**
- Initialize `notify::RecommendedWatcher`
- Watch root directory recursively
- Send events on `tx` channel
- What you learn: `notify` crate, channels in Rust (`tokio::sync::mpsc`)

**Function 5.2: `debounce_events(rx: Receiver<WatchEvent>, debounce_ms: u64) -> impl Stream<Item=Vec<PathBuf>>`**
- Collect events within debounce window
- Emit batched paths after quiet period
- What you learn: async streams, `tokio::time::sleep`, event batching

**Function 5.3: `reindex_file(path: &PathBuf, store: &AppState) -> Result<()>`**
- Check hash → if changed: chunk → content-hash each chunk → skip embedding for unchanged chunks (v2.2) → delete_by_file → insert new chunks → save hash
- Tantivy commit is NOT called here — batched per debounce window (v2.2)
- This is the hot path — must complete in <500ms for small files
- What you learn: measuring latency in Rust, `std::time::Instant`

**Function 5.4: `OllamaEmbedder::embed(texts: Vec<String>) -> Result<Vec<Vec<f32>>>`**
- HTTP POST to Ollama `/api/embeddings`
- Handle Ollama's specific request/response format
- What you learn: Ollama API, serde derive macros for request/response structs

**Function 5.5: `extract_imports(path: &Path, content: &str) -> Vec<String>`**
- Regex-based import extraction per language (fast, no full AST needed)
- Python: `import X`, `from X import`
- TypeScript/JS: `import ... from 'X'`, `require('X')`
- Rust: `use X::Y`
- Go: `import "X"`
- What you learn: `regex` crate in Rust, per-language dispatch

**Function 5.6: `DependencyGraph::add_file(from: &str, imports: Vec<String>) -> Result<()>`**
- Store in SQLite: `deps (from_file TEXT, to_file TEXT, PRIMARY KEY(from,to))`
- Resolve import strings to actual file paths in codebase
- What you learn: graph storage in relational DB, path resolution

**Function 5.7: `DependencyGraph::query(file: &str, direction: Direction) -> Vec<String>`**
- `Direction::Imports` → what this file imports
- `Direction::ImportedBy` → what imports this file
- Simple SQL queries against deps table
- What you learn: Rust enums with methods, SQL in Rust

**Function 5.8: `ProjectRegistry` — multi-project isolation**
- Map `sha256(abs_path)` → project key
- Each project gets own: LanceDB table, Tantivy index dir, SQLite DB, hash store
- What you learn: HashMap in Rust, structuring shared app state

**Function 5.9: `GitHistoryIndexer::index_commits(repo: &Path, limit: usize) -> Result<Vec<CommitInfo>>`**
- Parse git log via `git2` crate (libgit2 Rust bindings) — no shell exec
- Extract: commit hash, message, author, timestamp, changed file paths
- Index commit messages in Tantivy (BM25 searchable)
- Embed commit messages in LanceDB (semantic searchable)
- Limit to last N commits per branch (default: 500)
- What you learn: `git2` crate, working with git repositories programmatically

**`CommitInfo` struct:**
```rust
pub struct CommitInfo {
    pub hash: String,           // short commit hash (8 chars)
    pub message: String,        // full commit message
    pub author: String,
    pub timestamp: i64,         // unix timestamp
    pub changed_files: Vec<String>,  // files modified in this commit
}
```

**Function 5.10: `GitHistoryIndexer::search_commits(query: &str, top_k: usize) -> Result<Vec<CommitInfo>>`**
- Hybrid search across commit messages (same RRF fusion as code search)
- Enables queries like "show me commits related to authentication changes"
- What you learn: reusing search infrastructure for different data types

**Function 5.11: `FeedbackStore`** (v2.2.1 refined)
- Uses formal schema from Section 4.10 (`feedback.db`): `feedback` table with compound index on `(rel_path, chunk_key)`
- AST chunks keyed on `(rel_path, symbol_name)`, sliding-window on `(rel_path, line_bucket_{N})`
- `record(rel_path, chunk_key, useful: bool)` → store feedback
- `get_adjustment(rel_path, chunk_key) -> f32` → multiplier from aggregated feedback with 30-day decay:
  ```sql
  -- Fetch raw feedback records (typically <50 per chunk)
  SELECT useful, timestamp FROM feedback
  WHERE rel_path = ? AND chunk_key = ?;
  ```
  **Decay computed in Rust, not SQL (v2.3 fix):** SQLite's `power()` function requires
  `-DSQLITE_ENABLE_MATH_FUNCTIONS` at compile time, which `rusqlite` with `bundled` may not
  enable. Computing `0.5_f64.powf((now - ts) as f64 / 2_592_000.0)` in Rust is simpler,
  testable, and avoids build-flag dependencies. Typical result sets are <50 rows — no
  performance concern.
  ```sql
  -- Pruning: periodically delete feedback older than 90 days (3 half-lives = <12.5% weight)
  DELETE FROM feedback WHERE timestamp < strftime('%s','now') - 7776000;
  ```
- After 10+ signals: >70% useful → 1.15x boost, <30% useful → 0.87x penalty (v2.2 fix)
- Auto-invalidate feedback when chunk content_hash changes
- Integrated with ContextAssembler ranking pipeline
- What you learn: SQLite aggregation queries, exponential decay, feedback loop design

**Function 5.12: `ContextStitcher::diversify_languages(chunks: Vec<ContextChunk>) -> Vec<ContextChunk>`** (v2.1)
- After initial ranking, check language distribution of selected chunks
- Adaptive threshold: 70% for full-stack/monorepo, 95% for single-language, 90% default (v2.2 fix)
- When triggered: boost underrepresented languages by 1.2x, re-sort by adjusted scores
- Ensures full-stack queries return cross-language context
- What you learn: statistical distribution analysis, adaptive scoring

**Function 5.13: `CrossEncoderReranker::rerank(query: &str, results: Vec<SearchResult>) -> Vec<SearchResult>`** (v2.3 — moved from Phase 3)
- Load cross-encoder ONNX model (e.g., `cross-encoder/ms-marco-MiniLM-L-6-v2`, 22M params)
- Re-rank top-20 bi-encoder results by scoring each (query, passage) pair
- Re-sort results by cross-encoder score, return top-K
- ~10ms per pair on CPU → ~200ms for top-20 re-ranking (acceptable within search budget)
- **This is the single highest-impact accuracy improvement:** 10-15% Precision@5 lift over
  bi-encoder alone. Moved from Phase 3 to Phase 2 because shipping two phases without
  re-ranking means materially worse accuracy than necessary.
- What you learn: cross-encoder vs bi-encoder architecture, re-ranking pattern, ONNX multi-model

---

### Phase 3 — Production Hardening (Weeks 13–16)

Goal: Extended language support, packaging, benchmarks, advanced retrieval.

**Function 6.2: Extend tree-sitter grammars**
- Add Java, C, C++, C#, Ruby, PHP grammars to `Cargo.toml`
- Extend `detect_language()` and `CHUNK_NODE_TYPES` for each
- What you learn: Cargo feature flags, conditional compilation

**Function 6.3: `install.sh` + binary packaging**
- GitHub Actions workflow: cross-compile for macOS (x86_64 + aarch64), Linux (x86_64 + aarch64)
- `curl -fsSL https://install.vektor.dev | sh`
- What you learn: Rust cross-compilation, GitHub Actions, shell scripting

**Function 6.4: Benchmark suite**
- Compare Vektor vs Zilliz MCP vs code-memory on Precision@5
- Use public codebases as test fixtures (e.g., CPython, Tokio, Axum)
- Publish results in `BENCHMARKS.md`

**Function 6.5: Supermemory `code-chunk` chunking comparison**
- Benchmark Vektor's tree-sitter AST chunker against Supermemory's open-source
  [`code-chunk`](https://github.com/supermemoryai/code-chunk) npm package
- `code-chunk` claims Recall@5 of 70.1% vs 42.4% for fixed-size chunking (28-point improvement)
  using tree-sitter with scope hierarchy, imports, and sibling context enrichment
- **Test methodology:**
  1. Select 3 public codebases (one per language: Python, TypeScript, Rust)
  2. Create 50 ground-truth queries per codebase with expected relevant chunks (manually labeled)
  3. Run both chunkers on the same codebases, embed chunks with the same model (Jina v2 Base Code)
  4. Compare Recall@5 and Precision@5 on identical queries
  5. Measure: chunk count, avg chunk size, embedding cost (total tokens), retrieval accuracy
- **What to validate:**
  - Does Vektor's header preservation (v2.3) + sub-chunking match or beat `code-chunk`'s
    scope hierarchy enrichment?
  - Does Vektor's doc-file-specific windowing (40-line, 40% overlap) outperform `code-chunk`
    on mixed code+docs queries?
  - Does `code-chunk`'s sibling context (adjacent functions) improve retrieval for
    "what other functions relate to X?" queries? If so, consider adopting as a chunk metadata field.
- **Key differences to account for:**
  - `code-chunk` supports 6 languages (TS, JS, Python, Rust, Go, Java); Vektor Phase 1 covers 5
  - `code-chunk` is chunking-only (no search, no context assembly); comparison is on chunking quality only
  - Vektor's advantage is the full pipeline: chunking → embedding → hybrid search → context assembly
- Publish comparison in `BENCHMARKS.md` alongside Function 6.4 results

---

## 12. Risks and Mitigations

| Risk | Severity | Mitigation |
|---|---|---|
| `rmcp` Rust SDK API churn (v0.16, pre-1.0) | Medium | Pin version in Cargo.toml. `rmcp` is official MCP SDK, actively maintained. Fallback: implement stdio JSON-RPC manually (~200 lines) |
| ONNX model download on first run (~130–400MB) | Low | Progress bar via `indicatif`. Bundle smallest model (bge-small, 130MB). |
| LanceDB at scale (500K+ chunks) | Low | LanceDB handles millions of rows. Monitor memory; partition by project already isolates data. |
| OpenAI API rate limits during large initial index | Low | Configurable batch size, exponential backoff, automatic ONNX fallback |
| tree-sitter grammars increase binary size | Low | Only 5 core grammars in Phase 1; others behind Cargo feature flags |
| Event storm from large refactors (100s of files changed at once) | Medium | Debounce 500ms, max concurrent re-index workers capped at 4 |
| Crash during large initial index | Medium | Per-file status tracking in HashStore (`pending`/`indexed`/`failed`). Resume from where it left off. |
| LanceDB Arrow dependency weight | Low | Arrow is a production dependency used by Databricks, Snowflake, etc. Acceptable binary size trade-off for reliability. |
| Anthropic ships native indexing in Claude Code | Low | Vektor remains valuable: local, open, multi-provider embeddings, dependency graph |
| Jina v2 Base Code model download (~300MB) larger than bge-small (~130MB) | Low | Progress bar via `indicatif`. Offer `--lite` flag for bge-small fallback. First download only. |
| Skills standard evolving (agentskills.io) | Low | SKILL.md is simple markdown — easy to update format. Minimal maintenance burden. |
| Two-tier indexing adds complexity to tool handlers | Medium | Clear `IndexPhase` enum. All handlers check status via `IndexStatusTracker` before choosing search mode. Well-tested state machine. |
| Cross-store inconsistency after crash (LanceDB written, Tantivy not) | Medium | Delete-then-insert with HashStore as source of truth. `delete_by_file` before re-insert during recovery. Separate SQLite files per concern. (v2.2) |
| fastembed-rs may not support Jina v2 Base Code at pinned version | Medium | **v2.3 fix:** `ort` + `tokenizers` is now the primary approach; `fastembed` is optional. Build `OnnxEmbedder` directly against `ort` for Jina v2 with manual tokenization, pooling, and normalization. (v2.3) |
| Embedding dimension mismatch when switching models | Medium | Store (model_name, dim) in project metadata. Detect mismatch → force clean re-index with user warning. (v2.2) |
| Arrow version coupling across lancedb and independent pins | High | **v2.3 fix:** Do not independently pin arrow versions. Let `lancedb` dictate via transitive dependency. Use `cargo tree -d` to detect duplicate arrow versions. (v2.3) |
| SQLite `power()` function unavailable in standard builds | High | **v2.3 fix:** Compute feedback decay in Rust, not SQL. Fetch raw records, apply `0.5_f64.powf()` in code. Avoids `-DSQLITE_ENABLE_MATH_FUNCTIONS` build flag dependency. (v2.3) |
| Monorepo cross-package noise at 200K+ files | High | **v2.3 fix:** `scope` parameter on `get_context_for_prompt` biases results to active package. Auto-detect workspace roots. (v2.3) |
| LanceDB ANN degradation at 100K+ chunks without explicit index | Medium | Build IVF_PQ index after initial indexing. Rebuild via `index_stats` churn tracking at 10% threshold (Section 4.2). Skip IVF_PQ for <50K chunks. (v2.2.1) |
| `report_context_quality` — agents won't voluntarily call this | Low | Phase 3: infer quality passively (re-query within 30s = unhelpful, file edit after result = helpful). For now, rely on explicit SKILL.md instructions. (v2.2) |

---

## 13. Success Metrics

### Phase 1 — Must Have (before Phase 2 starts)

- [ ] Claude Code can call `index_codebase` on a 5,000-file Python repo and complete in <90s
- [ ] `search_code` returns semantically relevant results — manual check: >7/10 relevant in top-5
- [ ] **`get_context_for_prompt` returns token-budgeted, deduplicated context packages**
- [ ] **Context assembly overhead <10ms (dedup + budget + format)**
- [ ] **Related-file expansion includes imports and test files**
- [ ] **Query cache returns cached results in <5ms**
- [ ] Binary runs without any additional installation on macOS and Linux
- [ ] Zero external API key required for basic operation (local ONNX mode works)
- [ ] All 5 core MCP tools working: `index_codebase`, `search_code`, `get_context_for_prompt`, `get_index_status`, `clear_index`
- [ ] Keyword search available within 5s of first tool call on new project (two-tier indexing)
- [ ] Jina v2 Base Code model loads and embeds correctly via `ort` + `tokenizers` (v2.3)
- [ ] Skills file discoverable by Claude Code without explicit MCP config
- [ ] All tool responses include `index_status` and `index_coverage_pct` fields
- [ ] Content-addressed chunk IDs stable when function moves lines but content unchanged (v2.2)
- [ ] Delete-then-insert produces zero orphaned chunks after file deletion (v2.2)
- [ ] Synonym expansion improves recall: "auth" query finds "authentication" results (v2.2)
- [ ] ONNX warm-up completes during server startup — first query responds in <150ms (v2.2)
- [ ] Graceful shutdown flushes all pending writes (v2.2)
- [ ] `vektor serve` logs which ONNX execution provider was selected (v2.2)

### Phase 2 — Should Have

- [ ] File-save to searchable latency <500ms for single file re-index
- [ ] OpenAI-compatible embedding backend working with automatic fallback to local ONNX
- [ ] **Git history search: find commits related to a query**
- [ ] Precision@5 > 0.72 on code retrieval benchmark (vs Zilliz baseline ~0.65)
- [ ] Dependency graph correctly resolves Python and TypeScript imports
- [ ] Two different projects can be indexed simultaneously without collision
- [ ] `report_context_quality` stores and retrieves feedback correctly
- [ ] Recency weighting ranks recently modified files higher WITHOUT polluting top-5 with irrelevant recent files (v2.2)
- [ ] Feedback keyed on (rel_path, symbol_name) survives re-indexing — no orphaned feedback (v2.2)
- [ ] Embedding dimension mismatch detected on startup with clear user message (v2.2)
- [ ] Cross-encoder re-ranking improves Precision@5 to >0.78 (v2.3 — moved from Phase 3)
- [ ] `result_confidence` field accurately reflects result quality (v2.3)
- [ ] `scope` parameter correctly biases monorepo results to scoped package (v2.3)
- [ ] Chunk-level expansion returns only relevant chunks from related files, not entire files (v2.3)
- [ ] Language-specific token estimation error <15% across Python, Rust, Go, TypeScript (v2.3)
- [ ] Header preservation: sub-chunked functions include parent signature in each sub-chunk (v2.3)

### Phase 3 — Nice to Have

- [ ] Install script works: `curl -fsSL https://install.vektor.dev | sh`
- [ ] Benchmark results published and honest comparison vs Zilliz MCP documented
- [ ] 100+ GitHub stars within 30 days of public launch

### Competitive Benchmark (v2.1, updated v2.3)

- [ ] Vektor vs CocoIndex Code: same codebase, same queries — measure Precision@5 and context quality
- [ ] Vektor context assembly produces higher-quality agent responses than CocoIndex raw search (manual A/B evaluation)
- [ ] Time-to-first-result on fresh project: Vektor <8s vs CocoIndex baseline
- [ ] Vektor AST chunker vs Supermemory `code-chunk`: Recall@5 ≥ 70% on shared test codebases (matching or beating `code-chunk`'s published 70.1%)
- [ ] Vektor chunking produces fewer total chunks than `code-chunk` for same codebase (lower embedding cost) while maintaining equal or better recall

---

## 14. Implementation Rules

These rules exist so you learn Rust properly and the codebase stays clean.

### One function at a time
Do not move to Function 1.3 until Function 1.2 is:
- Written
- Understood line by line
- Tested with a simple `#[test]`
- Committed

### No AI slop rules
- Every function must be small enough to read in one screen (<50 lines ideally)
- Every function does exactly one thing — named precisely for what it does
- No magic numbers — use named constants
- No `.unwrap()` in production paths — use `?` and proper error types
- Every public function has a doc comment explaining what it does, inputs, and outputs

### Test every function
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_file_consistent() {
        // same file hashed twice = same result
    }

    #[test]
    fn test_hash_file_detects_change() {
        // modify file content = different hash
    }
}
```

### Git discipline
- One commit per function (or per test)
- Commit message format: `feat(chunker): add extract_chunks_ast for Python`
- Never commit broken code — `cargo check` must pass before every commit

### Ask when stuck
When you hit a Rust concept you don't understand (ownership, lifetimes, `Pin<Box<dyn Future>>`),
stop and ask Claude to explain it with a minimal example before continuing.
Understanding > speed.

### Module structure
```
vektor/
├── Cargo.toml
├── VEKTOR_PRD.md          ← this file
├── BENCHMARKS.md          ← added in Phase 3
├── README.md
└── src/
    ├── main.rs            ← CLI entry + tokio runtime (one-shot + serve modes)
    ├── config.rs          ← Config struct + TOML parsing + env var override
    ├── error.rs           ← Custom error types with thiserror
    ├── watcher.rs         ← File watching (Phase 2)
    ├── chunker/
    │   ├── mod.rs         ← chunk_file() dispatcher
    │   ├── ast.rs         ← tree-sitter AST chunking
    │   └── sliding.rs     ← fallback sliding window
    ├── embedder/
    │   ├── mod.rs         ← Embedder trait + build_embedder()
    │   ├── onnx.rs        ← OnnxEmbedder (local, default)
    │   ├── openai.rs      ← OpenAiCompatEmbedder (cloud, any compatible API)
    │   └── ollama.rs      ← OllamaEmbedder (local, requires Ollama running)
    ├── store/
    │   ├── mod.rs
    │   ├── vector.rs      ← VectorStore (LanceDB embedded)
    │   ├── text.rs        ← TextIndex (Tantivy)
    │   └── state.rs       ← HashStore (SQLite)
    ├── context/                ← THE DIFFERENTIATOR
    │   ├── mod.rs         ← ContextAssembler orchestrator
    │   ├── budget.rs      ← TokenCounter + budget allocation
    │   ├── dedup.rs       ← Deduplicator (merge overlapping chunks)
    │   ├── expander.rs    ← RelatedExpander (find related files)
    │   ├── cache.rs       ← QueryCache (LRU with TTL)
    │   ├── recency.rs     ← RecencyTracker (mtime-based scoring, v2.1)
    │   ├── stitcher.rs    ← ContextStitcher (multi-language diversity, v2.1)
    │   ├── feedback.rs    ← FeedbackStore (quality feedback loop, v2.1)
    │   └── synonyms.rs   ← SynonymExpander (static code concept map, v2.2)
    ├── search.rs          ← rrf_fuse() + search_hybrid() + AdaptiveWeights (v2.2)
    ├── reranker.rs        ← CrossEncoderReranker (Phase 2, moved from Phase 3 in v2.3)
    ├── indexer.rs         ← index_codebase() orchestrator
    ├── shallow.rs         ← ShallowIndexer (two-tier fast path, v2.1)
    ├── status.rs          ← IndexStatusTracker (index phase state machine, v2.1)
    ├── deps.rs            ← DependencyGraph (Phase 2)
    ├── git.rs             ← GitHistoryIndexer (Phase 2)
    └── mcp/
        ├── mod.rs         ← MCP server bootstrap
        └── handlers.rs    ← Tool handler functions (8 tools)
```

---

*Vektor — Not just search. Context.*
