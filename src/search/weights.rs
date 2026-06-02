/// Semantic/BM25 weights selected from query identifier density.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdaptiveWeights {
    pub semantic: f32,
    pub keyword: f32,
}

impl AdaptiveWeights {
    pub const IDENTIFIER_HEAVY: Self = Self {
        semantic: 0.4,
        keyword: 0.6,
    };
    pub const MIXED: Self = Self {
        semantic: 0.6,
        keyword: 0.4,
    };
    pub const NATURAL_LANGUAGE: Self = Self {
        semantic: 0.7,
        keyword: 0.3,
    };

    /// Compute weights using PRD section 4.2 density classification.
    pub fn compute(query: &str) -> Self {
        let tokens = identifier_tokens(query);
        if tokens.is_empty() {
            return Self::MIXED;
        }

        let identifier_count = tokens.iter().filter(|token| is_identifier(token)).count();
        let density = identifier_count as f32 / tokens.len() as f32;

        if density > 0.60 {
            Self::IDENTIFIER_HEAVY
        } else if density < 0.25 {
            Self::NATURAL_LANGUAGE
        } else {
            Self::MIXED
        }
    }

    pub fn sum(self) -> f32 {
        self.semantic + self.keyword
    }
}

fn identifier_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in query.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
            current.push(ch);
        } else {
            push_token(&mut tokens, &mut current);
        }
    }
    push_token(&mut tokens, &mut current);

    tokens
}

fn push_token(tokens: &mut Vec<String>, current: &mut String) {
    let token = current.trim_matches('.');
    if !token.is_empty() {
        tokens.push(token.to_owned());
    }
    current.clear();
}

fn is_identifier(token: &str) -> bool {
    is_snake_case(token)
        || is_camel_or_pascal_case(token)
        || is_dotted_path(token)
        || is_acronym_style(token)
}

fn is_snake_case(token: &str) -> bool {
    if !token.contains('_') {
        return false;
    }

    let mut saw_alpha = false;
    let mut saw_separator = false;
    for part in token.split('_') {
        if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            return false;
        }
        saw_alpha |= part.chars().any(|ch| ch.is_ascii_alphabetic());
        saw_separator = true;
    }

    saw_separator && saw_alpha
}

fn is_camel_or_pascal_case(token: &str) -> bool {
    let mut previous: Option<char> = None;
    for ch in token.chars() {
        if !ch.is_ascii_alphanumeric() {
            return false;
        }
        if let Some(prev) = previous
            && prev.is_ascii_lowercase()
            && ch.is_ascii_uppercase()
        {
            return true;
        }
        previous = Some(ch);
    }

    false
}

fn is_dotted_path(token: &str) -> bool {
    if !token.contains('.') {
        return false;
    }

    let mut saw_alpha = false;
    for part in token.split('.') {
        if part.is_empty()
            || !part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return false;
        }
        saw_alpha |= part.chars().any(|ch| ch.is_ascii_alphabetic());
    }

    saw_alpha
}

fn is_acronym_style(token: &str) -> bool {
    let body = token
        .strip_suffix('s')
        .filter(|base| base.chars().any(|ch| ch.is_ascii_uppercase()))
        .unwrap_or(token);

    let mut uppercase_count = 0;
    for ch in body.chars() {
        if ch.is_ascii_uppercase() {
            uppercase_count += 1;
        } else if !(ch.is_ascii_digit() || ch == '_') {
            return false;
        }
    }

    uppercase_count >= 2
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn assert_weights(actual: AdaptiveWeights, expected: AdaptiveWeights) {
        assert_eq!(actual, expected);
        assert!(
            (actual.sum() - 1.0).abs() < f32::EPSILON,
            "weights must sum to 1.0: {actual:?}"
        );
    }

    #[test]
    fn documented_examples_return_expected_weights() {
        assert_weights(
            AdaptiveWeights::compute("validate_token AuthMiddleware"),
            AdaptiveWeights::IDENTIFIER_HEAVY,
        );
        assert_weights(
            AdaptiveWeights::compute("how does authentication work"),
            AdaptiveWeights::NATURAL_LANGUAGE,
        );
        assert_weights(
            AdaptiveWeights::compute("how does validate_token handle expired JWTs"),
            AdaptiveWeights::MIXED,
        );
    }

    #[test]
    fn pascal_case_counts_as_identifier_token() {
        assert_weights(
            AdaptiveWeights::compute("validate_token AuthMiddleware"),
            AdaptiveWeights::IDENTIFIER_HEAVY,
        );
    }

    #[test]
    fn acronym_plural_counts_as_identifier_token() {
        assert_weights(
            AdaptiveWeights::compute("how does validate_token handle expired JWTs"),
            AdaptiveWeights::MIXED,
        );
    }

    #[test]
    fn one_identifier_in_natural_language_does_not_flip_keyword_heavy() {
        assert_weights(
            AdaptiveWeights::compute("how does validate_token work"),
            AdaptiveWeights::MIXED,
        );
    }

    #[test]
    fn natural_language_with_many_words_stays_semantic_heavy() {
        assert_weights(
            AdaptiveWeights::compute("explain how authentication and sessions work"),
            AdaptiveWeights::NATURAL_LANGUAGE,
        );
    }

    #[test]
    fn dotted_paths_count_as_identifier_tokens() {
        assert_weights(
            AdaptiveWeights::compute("crate.search.rrf validateToken"),
            AdaptiveWeights::IDENTIFIER_HEAVY,
        );
    }

    #[test]
    fn empty_query_uses_mixed_default() {
        assert_weights(AdaptiveWeights::compute(" \t\n "), AdaptiveWeights::MIXED);
    }
}
