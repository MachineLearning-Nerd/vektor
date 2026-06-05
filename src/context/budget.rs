use std::sync::OnceLock;

use crate::error::{Result, VektorError};

#[allow(dead_code)]
static CL100K_BASE: OnceLock<std::result::Result<tiktoken_rs::CoreBPE, String>> = OnceLock::new();

#[allow(dead_code)]
pub(crate) struct TokenCounter;

#[allow(dead_code)]
impl TokenCounter {
    pub(crate) fn estimate(text: &str, language: &str) -> usize {
        if text.is_empty() {
            return 0;
        }

        ((text.len() as f64) / ratio_for(language)).ceil() as usize
    }

    pub(crate) fn count_exact(text: &str) -> Result<usize> {
        let encoder = CL100K_BASE
            .get_or_init(|| tiktoken_rs::cl100k_base().map_err(|error| error.to_string()))
            .as_ref()
            .map_err(|error| {
                VektorError::Embedding(format!("failed to initialize cl100k_base encoder: {error}"))
            })?;

        Ok(encoder.encode_ordinary(text).len())
    }
}

#[allow(dead_code)]
fn ratio_for(language: &str) -> f64 {
    match language.trim().to_ascii_lowercase().as_str() {
        "python" | "javascript" | "typescript" | "jsx" | "tsx" => 3.5,
        "rust" | "go" | "java" => 4.2,
        "markdown" | "md" | "rst" => 4.8,
        _ => 3.8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_is_language_specific() {
        let text = "x".repeat(420);

        let python = TokenCounter::estimate(&text, "python");
        let rust = TokenCounter::estimate(&text, "rust");
        let markdown = TokenCounter::estimate(&text, "markdown");

        assert!(python > rust);
        assert!(rust > markdown);
        assert_eq!(python, 120);
        assert_eq!(rust, 100);
        assert_eq!(markdown, 88);
    }

    #[test]
    fn estimate_unknown_language_uses_default() {
        assert_eq!(TokenCounter::estimate(&"x".repeat(380), ""), 100);
        assert_eq!(TokenCounter::estimate(&"x".repeat(380), "zig"), 100);
    }

    #[test]
    fn estimate_nonempty_never_zero() {
        assert_eq!(TokenCounter::estimate("", "rust"), 0);
        assert_eq!(TokenCounter::estimate("x", "rust"), 1);
    }

    #[test]
    fn count_exact_matches_tiktoken_golden() {
        let expected = tiktoken_rs::cl100k_base()
            .expect("cl100k_base encoder")
            .encode_ordinary("fn main() { println!(\"hello\"); }")
            .len();

        assert_eq!(
            TokenCounter::count_exact("fn main() { println!(\"hello\"); }").expect("exact count"),
            expected
        );
    }
}
