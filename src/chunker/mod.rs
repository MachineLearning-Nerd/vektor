use std::path::Path;

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
}
