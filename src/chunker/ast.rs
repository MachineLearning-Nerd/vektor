use tree_sitter::{Parser, Tree};

use crate::{
    chunker::Language,
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
}
