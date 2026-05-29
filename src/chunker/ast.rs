use tree_sitter::{Node, Parser, Tree};

use crate::{
    chunker::{Chunk, Language},
    config::Config,
    error::{Result, VektorError},
};

#[allow(dead_code)]
pub fn parse_ast(content: &str, language: Language) -> Result<Tree> {
    let grammar = tree_sitter_language(language);
    let mut parser = Parser::new();
    parser.set_language(&grammar).map_err(|error| {
        VektorError::Parse(format!(
            "failed to load {} tree-sitter grammar: {error}",
            language.as_str()
        ))
    })?;

    parser.parse(content, None).ok_or_else(|| {
        VektorError::Parse(format!(
            "tree-sitter returned no parse tree for {}",
            language.as_str()
        ))
    })
}

#[allow(dead_code)]
pub fn extract_chunks_ast(
    tree: &Tree,
    content: &str,
    rel_path: &str,
    language: Language,
    config: &Config,
) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    collect_chunks(
        tree.root_node(),
        content,
        rel_path,
        language,
        config,
        &mut chunks,
    );
    chunks.sort_by_key(|chunk| (chunk.start_line, chunk.end_line));
    chunks
}

fn tree_sitter_language(language: Language) -> tree_sitter::Language {
    match language {
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::JavaScript | Language::Jsx => tree_sitter_javascript::LANGUAGE.into(),
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
    }
}

fn collect_chunks(
    node: Node<'_>,
    content: &str,
    rel_path: &str,
    language: Language,
    config: &Config,
    chunks: &mut Vec<Chunk>,
) -> bool {
    if is_chunk_node(node, language) {
        if should_decompose_container(node, language) {
            let before = chunks.len();
            collect_child_chunks(node, content, rel_path, language, config, chunks);
            if chunks.len() > before {
                return true;
            }
        }

        emit_node_chunks(node, content, rel_path, language, config, chunks);
        return true;
    }

    collect_child_chunks(node, content, rel_path, language, config, chunks)
}

fn collect_child_chunks(
    node: Node<'_>,
    content: &str,
    rel_path: &str,
    language: Language,
    config: &Config,
    chunks: &mut Vec<Chunk>,
) -> bool {
    let before = chunks.len();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            collect_chunks(child, content, rel_path, language, config, chunks);
        }
    }
    chunks.len() > before
}

fn is_chunk_node(node: Node<'_>, language: Language) -> bool {
    match language {
        Language::Python => matches!(node.kind(), "function_definition" | "class_definition"),
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => matches!(
            node.kind(),
            "function_declaration" | "method_definition" | "class_declaration"
        ),
        Language::Rust => matches!(node.kind(), "function_item" | "impl_item" | "struct_item"),
        Language::Go => matches!(node.kind(), "function_declaration" | "method_declaration"),
    }
}

fn should_decompose_container(node: Node<'_>, language: Language) -> bool {
    match language {
        Language::Python => node.kind() == "class_definition",
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            node.kind() == "class_declaration"
        }
        Language::Rust => node.kind() == "impl_item",
        Language::Go => false,
    }
}

fn emit_node_chunks(
    node: Node<'_>,
    content: &str,
    rel_path: &str,
    language: Language,
    config: &Config,
    chunks: &mut Vec<Chunk>,
) {
    let node_content = node_text(node, content);
    let lines: Vec<&str> = node_content.lines().collect();
    if lines.is_empty() {
        return;
    }

    let start_line = node.start_position().row + 1;
    let max_lines = config.index.chunk_max_lines.max(1);
    let symbol_name = symbol_name(node, content);
    let symbol_type = Some(node.kind().to_string());
    let line_count = lines.len();

    if line_count <= max_lines {
        chunks.push(Chunk::new(
            node_content,
            rel_path.to_string(),
            start_line,
            start_line + line_count - 1,
            symbol_name,
            symbol_type,
            Some(language),
        ));
        return;
    }

    let overlap = ((max_lines * usize::from(config.index.chunk_overlap_pct)) / 100)
        .min(max_lines.saturating_sub(1));
    let step = (max_lines - overlap).max(1);
    let header = lines[..lines.len().min(5)].join("\n");
    let mut window_start = 0;
    let mut ordinal = 0;

    while window_start < lines.len() {
        let window_end = (window_start + max_lines).min(lines.len());
        let mut chunk_content = lines[window_start..window_end].join("\n");
        if ordinal > 0 && !header.is_empty() {
            chunk_content = format!("{header}\n{chunk_content}");
        }

        chunks.push(Chunk::new(
            chunk_content,
            rel_path.to_string(),
            start_line + window_start,
            start_line + window_end - 1,
            symbol_name.clone(),
            symbol_type.clone(),
            Some(language),
        ));

        if window_end == lines.len() {
            break;
        }
        window_start += step;
        ordinal += 1;
    }
}

fn node_text(node: Node<'_>, content: &str) -> String {
    content[node.start_byte()..node.end_byte()].to_string()
}

fn symbol_name(node: Node<'_>, content: &str) -> Option<String> {
    node.child_by_field_name("name")
        .or_else(|| {
            first_named_child(
                node,
                &["identifier", "property_identifier", "type_identifier"],
            )
        })
        .and_then(|child| child.utf8_text(content.as_bytes()).ok())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn first_named_child<'tree>(node: Node<'tree>, kinds: &[&str]) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.is_named() && kinds.contains(&child.kind()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ast_parses_valid_fixtures_for_supported_languages() {
        let cases = [
            (
                Language::Python,
                "def add(x):\n    return x + 1\n",
                "module",
            ),
            (
                Language::TypeScript,
                "function add(x: number): number { return x + 1; }\n",
                "program",
            ),
            (
                Language::Tsx,
                "export function View() { return <div>{value}</div>; }\n",
                "program",
            ),
            (
                Language::JavaScript,
                "function add(x) { return x + 1; }\n",
                "program",
            ),
            (
                Language::Jsx,
                "export function View() { return <div>{value}</div>; }\n",
                "program",
            ),
            (
                Language::Rust,
                "fn add(x: i32) -> i32 { x + 1 }\n",
                "source_file",
            ),
            (
                Language::Go,
                "package main\nfunc add(x int) int { return x + 1 }\n",
                "source_file",
            ),
        ];

        for (language, content, root_kind) in cases {
            let tree =
                parse_ast(content, language).unwrap_or_else(|_| panic!("{}", language.as_str()));
            let root = tree.root_node();

            assert_eq!(root.kind(), root_kind, "{}", language.as_str());
            assert!(!root.has_error(), "{}", language.as_str());
        }
    }

    #[test]
    fn parse_ast_preserves_syntax_errors_for_dispatcher_fallback() {
        let tree = parse_ast("def broken(:\n", Language::Python).expect("parse syntax-error tree");

        assert!(tree.root_node().has_error());
    }

    #[test]
    fn ast_chunk_extracts_semantic_symbols_for_supported_languages() {
        let cases = [
            (
                Language::Python,
                "class Service:\n    def run(self):\n        return 1\n\ndef helper():\n    return 2\n",
                "src/service.py",
                vec![
                    ("run", "function_definition"),
                    ("helper", "function_definition"),
                ],
            ),
            (
                Language::TypeScript,
                "class Service { run(): number { return 1; } }\nfunction helper(): number { return 2; }\n",
                "src/service.ts",
                vec![
                    ("run", "method_definition"),
                    ("helper", "function_declaration"),
                ],
            ),
            (
                Language::JavaScript,
                "class Service { run() { return 1; } }\nfunction helper() { return 2; }\n",
                "src/service.js",
                vec![
                    ("run", "method_definition"),
                    ("helper", "function_declaration"),
                ],
            ),
            (
                Language::Rust,
                "struct Service { value: i32 }\nimpl Service { fn run(&self) -> i32 { self.value } }\nfn helper() -> i32 { 2 }\n",
                "src/service.rs",
                vec![
                    ("Service", "struct_item"),
                    ("run", "function_item"),
                    ("helper", "function_item"),
                ],
            ),
            (
                Language::Go,
                "package main\ntype Service struct{}\nfunc helper() int { return 2 }\nfunc (s Service) Run() int { return 1 }\n",
                "service.go",
                vec![
                    ("helper", "function_declaration"),
                    ("Run", "method_declaration"),
                ],
            ),
        ];

        for (language, content, rel_path, expected_symbols) in cases {
            let tree = parse_ast(content, language).expect("parse fixture");
            let chunks = extract_chunks_ast(&tree, content, rel_path, language, &Config::default());
            let actual_symbols: Vec<_> = chunks
                .iter()
                .map(|chunk| {
                    (
                        chunk.symbol_name.as_deref().unwrap_or(""),
                        chunk.symbol_type.as_deref().unwrap_or(""),
                    )
                })
                .collect();

            assert_eq!(actual_symbols, expected_symbols, "{}", language.as_str());
            for chunk in chunks {
                assert_eq!(chunk.rel_path, rel_path);
                assert_eq!(chunk.language, Some(language));
                assert_eq!(chunk.content_hash.len(), 64);
                assert!(chunk.start_line <= chunk.end_line);
                assert!(!chunk.content.is_empty());
            }
        }
    }

    #[test]
    fn ast_chunk_splits_oversized_function_and_prepends_header() {
        let mut content = String::from("def huge():\n");
        for line in 1..250 {
            content.push_str(&format!("    value_{line} = {line}\n"));
        }
        let mut config = Config::default();
        config.index.chunk_max_lines = 200;

        let tree = parse_ast(&content, Language::Python).expect("parse oversized fixture");
        let chunks = extract_chunks_ast(&tree, &content, "src/huge.py", Language::Python, &config);

        assert_eq!(chunks.len(), 2);
        assert_eq!((chunks[0].start_line, chunks[0].end_line), (1, 200));
        assert_eq!((chunks[1].start_line, chunks[1].end_line), (151, 250));
        assert!(chunks[1].content.starts_with("def huge():\n"));
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.end_line - chunk.start_line < config.index.chunk_max_lines)
        );
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.symbol_name.as_deref() == Some("huge"))
        );
    }
}
