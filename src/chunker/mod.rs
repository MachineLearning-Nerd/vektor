use std::path::Path;

use crate::config::Config;

pub mod ast;
pub mod sliding;

#[allow(unused_imports)]
pub use ast::parse_ast;
#[allow(unused_imports)]
pub use sliding::extract_chunks_sliding;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Python,
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
    Rust,
    Go,
}

#[allow(dead_code)]
impl Language {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
            Self::Jsx => "jsx",
            Self::Rust => "rust",
            Self::Go => "go",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub id: String,
    pub content: String,
    pub content_hash: String,
    pub rel_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub symbol_name: Option<String>,
    pub symbol_type: Option<String>,
    pub language: Option<Language>,
}

#[allow(dead_code)]
impl Chunk {
    pub fn new(
        content: String,
        rel_path: String,
        start_line: usize,
        end_line: usize,
        symbol_name: Option<String>,
        symbol_type: Option<String>,
        language: Option<Language>,
    ) -> Self {
        let content_hash = crate::state::hash_content(&content);

        Self {
            id: String::new(),
            content,
            content_hash,
            rel_path,
            start_line,
            end_line,
            symbol_name,
            symbol_type,
            language,
        }
    }
}

#[allow(dead_code)]
pub fn chunk_file(path: &Path, content: &str, config: &Config) -> Vec<Chunk> {
    let rel_path = normalized_path(path);
    let language = detect_language(path);
    let mut chunks = match language {
        Some(language) => match ast::parse_ast(content, language) {
            Ok(tree) if tree.root_node().has_error() => {
                tracing::debug!(
                    path = %path.display(),
                    language = language.as_str(),
                    "falling back to sliding chunks after syntax errors"
                );
                sliding::extract_chunks_sliding(content, &rel_path, Some(language), config)
            }
            Ok(tree) => {
                let ast_chunks =
                    ast::extract_chunks_ast(&tree, content, &rel_path, language, config);
                if ast_chunks.is_empty() {
                    tracing::debug!(
                        path = %path.display(),
                        language = language.as_str(),
                        "falling back to sliding chunks after empty AST chunks"
                    );
                    sliding::extract_chunks_sliding(content, &rel_path, Some(language), config)
                } else {
                    ast_chunks
                }
            }
            Err(error) => {
                tracing::debug!(
                    path = %path.display(),
                    language = language.as_str(),
                    error = %error,
                    "falling back to sliding chunks after parser failure"
                );
                sliding::extract_chunks_sliding(content, &rel_path, Some(language), config)
            }
        },
        None => sliding::extract_chunks_sliding(content, &rel_path, None, config),
    };

    assign_chunk_ids(&mut chunks);
    chunks
}

fn assign_chunk_ids(chunks: &mut [Chunk]) {
    for (index, chunk) in chunks.iter_mut().enumerate() {
        let key = if let Some(symbol_name) = chunk.symbol_name.as_deref() {
            format!("{}:{}:{}", chunk.rel_path, symbol_name, chunk.content_hash)
        } else {
            format!("{}:chunk_{}:{}", chunk.rel_path, index, chunk.content_hash)
        };
        chunk.id = crate::state::hash_content(&key);
    }
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[allow(dead_code)]
pub fn detect_language(path: &Path) -> Option<Language> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "py" => Some(Language::Python),
        "ts" => Some(Language::TypeScript),
        "tsx" => Some(Language::Tsx),
        "js" => Some(Language::JavaScript),
        "jsx" => Some(Language::Jsx),
        "rs" => Some(Language::Rust),
        "go" => Some(Language::Go),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_types_preserve_metadata_and_full_content_hash() {
        let chunk = Chunk::new(
            "fn main() {}\n".into(),
            "src/main.rs".into(),
            10,
            12,
            Some("main".into()),
            Some("function".into()),
            Some(Language::Rust),
        );

        assert_eq!(chunk.content, "fn main() {}\n");
        assert_eq!(
            chunk.content_hash,
            "536e506bb90914c243a12b397b9a998f85ae2cbd9ba02dfd03a9e155ca5ca0f4"
        );
        assert_eq!(chunk.id, "");
        assert_eq!(chunk.rel_path, "src/main.rs");
        assert_eq!(chunk.start_line, 10);
        assert_eq!(chunk.end_line, 12);
        assert_eq!(chunk.symbol_name.as_deref(), Some("main"));
        assert_eq!(chunk.symbol_type.as_deref(), Some("function"));
        assert_eq!(chunk.language, Some(Language::Rust));
    }

    #[test]
    fn language_detection_maps_supported_extensions() {
        let cases = [
            ("main.py", Some(Language::Python)),
            ("component.ts", Some(Language::TypeScript)),
            ("component.tsx", Some(Language::Tsx)),
            ("index.js", Some(Language::JavaScript)),
            ("view.jsx", Some(Language::Jsx)),
            ("lib.rs", Some(Language::Rust)),
            ("server.go", Some(Language::Go)),
        ];

        for (path, expected) in cases {
            assert_eq!(detect_language(Path::new(path)), expected, "{path}");
        }
    }

    #[test]
    fn language_detection_returns_none_for_unsupported_or_extensionless_paths() {
        for path in [
            "README.md",
            "Dockerfile",
            "Makefile",
            "script",
            "archive.tar.gz",
        ] {
            assert_eq!(detect_language(Path::new(path)), None, "{path}");
        }
    }

    #[test]
    fn chunk_file_uses_ast_for_supported_valid_source_and_stable_ids() {
        let content = "fn add(x: i32) -> i32 { x + 1 }\n";
        let first = chunk_file(Path::new("src/lib.rs"), content, &Config::default());
        let second = chunk_file(Path::new("src/lib.rs"), content, &Config::default());

        assert_eq!(first.len(), 1);
        assert_eq!(first[0].symbol_name.as_deref(), Some("add"));
        assert_eq!(first[0].symbol_type.as_deref(), Some("function_item"));
        assert_eq!(first[0].language, Some(Language::Rust));
        assert_eq!(first[0].id.len(), 64);
        assert_eq!(first[0].content_hash.len(), 64);
        assert_eq!(first[0].id, second[0].id);
        assert_eq!(first[0].content_hash, second[0].content_hash);
    }

    #[test]
    fn chunk_file_uses_sliding_for_unsupported_docs() {
        let content = numbered_lines(70);
        let chunks = chunk_file(Path::new("README.md"), &content, &Config::default());

        assert_eq!(line_ranges(&chunks), vec![(1, 40), (25, 64), (49, 70)]);
        assert!(chunks.iter().all(|chunk| chunk.language.is_none()));
        assert!(chunks.iter().all(|chunk| chunk.symbol_name.is_none()));
        assert!(chunks.iter().all(|chunk| chunk.id.len() == 64));
    }

    #[test]
    fn chunk_file_falls_back_to_sliding_on_syntax_errors() {
        let chunks = chunk_file(Path::new("broken.py"), "def broken(:\n", &Config::default());

        assert_eq!(line_ranges(&chunks), vec![(1, 1)]);
        assert_eq!(chunks[0].language, Some(Language::Python));
        assert_eq!(chunks[0].symbol_name, None);
        assert_eq!(chunks[0].symbol_type, None);
        assert_eq!(chunks[0].id.len(), 64);
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
