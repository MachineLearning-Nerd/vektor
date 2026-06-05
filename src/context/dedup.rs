use crate::search::hybrid::HybridResult;

#[allow(dead_code)]
pub(crate) struct Deduplicator;

#[allow(dead_code)]
impl Deduplicator {
    pub(crate) fn deduplicate(chunks: Vec<HybridResult>) -> Vec<HybridResult> {
        if chunks.is_empty() {
            return Vec::new();
        }

        let mut sortable = chunks
            .into_iter()
            .enumerate()
            .map(|(original_index, chunk)| DedupCandidate {
                original_index,
                chunk,
            })
            .collect::<Vec<_>>();
        sortable.sort_by(|left, right| {
            left.chunk
                .rel_path
                .cmp(&right.chunk.rel_path)
                .then_with(|| left.chunk.start_line.cmp(&right.chunk.start_line))
                .then_with(|| left.chunk.end_line.cmp(&right.chunk.end_line))
                .then_with(|| left.original_index.cmp(&right.original_index))
        });

        let mut merged = Vec::new();
        let mut current: Option<DedupCandidate> = None;
        for candidate in sortable {
            let Some(active) = current.as_mut() else {
                current = Some(candidate);
                continue;
            };

            if should_merge(&active.chunk, &candidate.chunk) {
                merge_into(active, candidate);
            } else {
                if let Some(done) = current.take() {
                    merged.push(done);
                }
                current = Some(candidate);
            }
        }

        if let Some(done) = current {
            merged.push(done);
        }

        merged.sort_by(|left, right| left.original_index.cmp(&right.original_index));
        merged
            .into_iter()
            .map(|candidate| candidate.chunk)
            .collect()
    }
}

struct DedupCandidate {
    original_index: usize,
    chunk: HybridResult,
}

fn should_merge(left: &HybridResult, right: &HybridResult) -> bool {
    if left.rel_path != right.rel_path {
        return false;
    }

    let overlap = overlap_lines(left, right);
    let smaller_span = min_span(left, right);
    overlap > 0 && smaller_span > 0 && overlap.saturating_mul(2) > smaller_span
}

fn overlap_lines(left: &HybridResult, right: &HybridResult) -> u64 {
    let start = left.start_line.max(right.start_line);
    let end = left.end_line.min(right.end_line);
    if end < start { 0 } else { end - start + 1 }
}

fn min_span(left: &HybridResult, right: &HybridResult) -> u64 {
    span(left).min(span(right))
}

fn span(chunk: &HybridResult) -> u64 {
    if chunk.end_line < chunk.start_line {
        0
    } else {
        chunk.end_line - chunk.start_line + 1
    }
}

fn merge_into(active: &mut DedupCandidate, candidate: DedupCandidate) {
    let start_line = active.chunk.start_line.min(candidate.chunk.start_line);
    let end_line = active.chunk.end_line.max(candidate.chunk.end_line);
    if should_replace(&active.chunk, &candidate.chunk) {
        *active = candidate;
    }
    active.chunk.start_line = start_line;
    active.chunk.end_line = end_line;
}

fn should_replace(active: &HybridResult, candidate: &HybridResult) -> bool {
    match candidate.relevance_score.total_cmp(&active.relevance_score) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => {
            span(candidate)
                .cmp(&span(active))
                .then_with(|| active.start_line.cmp(&candidate.start_line))
                == std::cmp::Ordering::Greater
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::hybrid::HybridResult;

    fn chunk(id: &str, rel_path: &str, start_line: u64, end_line: u64, score: f32) -> HybridResult {
        HybridResult {
            chunk_id: id.to_string(),
            rel_path: rel_path.to_string(),
            start_line,
            end_line,
            symbol_name: Some(format!("symbol_{id}")),
            symbol_type: Some("function".to_string()),
            language: "rust".to_string(),
            content: format!("in-memory content for {id}"),
            relevance_score: score,
            semantic_score: Some(score),
            keyword_score: None,
            last_modified: 1_700_000_000,
        }
    }

    #[test]
    fn merges_high_overlap_chunks() {
        let chunks = vec![
            chunk("lower", "src/lib.rs", 1, 10, 0.4),
            chunk("higher", "src/lib.rs", 5, 12, 0.9),
        ];

        let deduped = Deduplicator::deduplicate(chunks);

        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].chunk_id, "higher");
        assert_eq!(deduped[0].start_line, 1);
        assert_eq!(deduped[0].end_line, 12);
        assert_eq!(deduped[0].content, "in-memory content for higher");
    }

    #[test]
    fn keeps_distinct_low_overlap_neighbors() {
        let chunks = vec![
            chunk("first", "src/lib.rs", 1, 10, 0.9),
            chunk("second", "src/lib.rs", 6, 15, 0.8),
        ];

        let deduped = Deduplicator::deduplicate(chunks);

        assert_eq!(deduped.len(), 2);
        assert_eq!(
            deduped
                .iter()
                .map(|chunk| chunk.chunk_id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
    }

    #[test]
    fn never_merges_across_files() {
        let chunks = vec![
            chunk("left", "src/lib.rs", 1, 10, 0.9),
            chunk("right", "src/main.rs", 1, 10, 0.8),
        ];

        let deduped = Deduplicator::deduplicate(chunks);

        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn nested_chunk_is_absorbed() {
        let chunks = vec![
            chunk("container", "src/lib.rs", 1, 20, 0.7),
            chunk("nested", "src/lib.rs", 5, 10, 0.9),
        ];

        let deduped = Deduplicator::deduplicate(chunks);

        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].chunk_id, "nested");
        assert_eq!(deduped[0].start_line, 1);
        assert_eq!(deduped[0].end_line, 20);
    }

    #[test]
    fn adjacent_non_overlapping_kept() {
        let chunks = vec![
            chunk("first", "src/lib.rs", 1, 10, 0.9),
            chunk("second", "src/lib.rs", 11, 20, 0.8),
        ];

        let deduped = Deduplicator::deduplicate(chunks);

        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn no_disk_read_uses_in_memory_content() {
        let chunks = vec![
            chunk("lower", "missing/path.rs", 1, 10, 0.4),
            chunk("higher", "missing/path.rs", 5, 12, 0.9),
        ];

        let deduped = Deduplicator::deduplicate(chunks);

        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].content, "in-memory content for higher");
    }
}
