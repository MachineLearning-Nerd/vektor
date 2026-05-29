use crate::{chunker::Chunk, chunker::Language, config::Config};

const CODE_WINDOW_LINES: usize = 80;
const CODE_OVERLAP_PCT: usize = 25;
const DOC_OVERLAP_PCT: usize = 40;

#[allow(dead_code)]
pub fn extract_chunks_sliding(
    content: &str,
    rel_path: &str,
    language: Option<Language>,
    config: &Config,
) -> Vec<Chunk> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let (window_lines, overlap_pct) = window_settings(rel_path, config);
    let window_lines = window_lines.max(1);
    let overlap_lines = ((window_lines * overlap_pct) / 100).min(window_lines.saturating_sub(1));
    let step = (window_lines - overlap_lines).max(1);

    let mut chunks = Vec::new();
    let mut start = 0;
    while start < lines.len() {
        let end = (start + window_lines).min(lines.len());
        chunks.push(Chunk::new(
            lines[start..end].join("\n"),
            rel_path.to_string(),
            start + 1,
            end,
            None,
            None,
            language,
        ));

        if end == lines.len() {
            break;
        }
        start += step;
    }

    chunks
}

fn window_settings(rel_path: &str, config: &Config) -> (usize, usize) {
    if is_doc_path(rel_path) {
        (config.index.doc_chunk_max_lines, DOC_OVERLAP_PCT)
    } else {
        (CODE_WINDOW_LINES, CODE_OVERLAP_PCT)
    }
}

fn is_doc_path(rel_path: &str) -> bool {
    let Some((_, extension)) = rel_path.rsplit_once('.') else {
        return false;
    };

    matches!(
        extension.to_ascii_lowercase().as_str(),
        "md" | "txt" | "rst"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliding_code_uses_eighty_line_windows_with_twenty_five_percent_overlap() {
        let content = numbered_lines(100);
        let chunks = extract_chunks_sliding(
            &content,
            "src/generated.unknown",
            Some(Language::Rust),
            &Config::default(),
        );

        assert_eq!(line_ranges(&chunks), vec![(1, 80), (61, 100)]);
        assert_eq!(chunks[0].rel_path, "src/generated.unknown");
        assert_eq!(chunks[0].language, Some(Language::Rust));
        assert_eq!(chunks[0].symbol_name, None);
        assert_eq!(chunks[0].symbol_type, None);
        assert_eq!(chunks[0].content_hash.len(), 64);
        assert!(chunks[0].content.starts_with("line 1\n"));
        assert!(chunks[0].content.ends_with("line 80"));
    }

    #[test]
    fn sliding_docs_use_doc_window_and_forty_percent_overlap() {
        let content = numbered_lines(70);
        let chunks = extract_chunks_sliding(&content, "README.md", None, &Config::default());

        assert_eq!(line_ranges(&chunks), vec![(1, 40), (25, 64), (49, 70)]);
        assert_eq!(chunks[0].language, None);
    }

    #[test]
    fn sliding_handles_short_and_empty_files() {
        let short = extract_chunks_sliding("one\ntwo\n", "notes.txt", None, &Config::default());
        assert_eq!(line_ranges(&short), vec![(1, 2)]);
        assert_eq!(short[0].content, "one\ntwo");

        let empty = extract_chunks_sliding("", "empty.txt", None, &Config::default());
        assert!(empty.is_empty());
    }

    fn numbered_lines(count: usize) -> String {
        (1..=count)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn line_ranges(chunks: &[Chunk]) -> Vec<(usize, usize)> {
        chunks
            .iter()
            .map(|chunk| (chunk.start_line, chunk.end_line))
            .collect()
    }
}
