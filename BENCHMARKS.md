# Benchmarks

This file records the launch-quality retrieval baseline for Vektor.

## v0.4.0 launch baseline

| Field | Value |
|---|---|
| Date | 2026-06-06 |
| Source commit | `841a109` |
| Command | `PROTOC=/tmp/vektor-protoc/protoc-35.0/bin/protoc cargo bench --bench retrieval_quality` |
| Fixture | `benches/fixtures/tokio_20_queries.json` |
| Query count | 20 |
| Corpus | Checked-in Tokio fixture, 13 Rust files |
| Retrieval path | Vektor `TextIndex` BM25 over fixture chunks |
| Backend/model | Keyword-only benchmark; no embedding model, ONNX runtime, OpenAI API, or network access |
| Index config | `chunk_max_lines=80`, `chunk_overlap_pct=25`, `doc_chunk_max_lines=40`, `max_file_size_kb=1024` |
| Rust | `rustc 1.91.0 (f8297e351 2025-10-28)` |
| Cargo | `cargo 1.91.0 (ea2d97820 2025-10-10)` |
| Host | `aarch64-apple-darwin` |
| OS | `Darwin 25.4.0 arm64` |

| Metric | Baseline |
|---|---:|
| Precision@5 | 0.200 |
| Recall@5 | 1.000 |
| MRR | 0.975 |
| Elapsed | 80 ms |

## Reproduction

```bash
PROTOC=/tmp/vektor-protoc/protoc-35.0/bin/protoc cargo bench --bench retrieval_quality
```

Expected report shape:

```text
# Vektor retrieval benchmark

- Queries: 20
- Precision@5: 0.200
- Recall@5: 1.000
- MRR: 0.975
```

## PR delta policy

Benchmark deltas are informational for `v0.4.0`. CI may publish the delta in a
pull-request comment or workflow summary, but benchmark movement does not block
PRs until a later release defines calibrated regression thresholds.
