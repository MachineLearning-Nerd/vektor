/// Expands BM25 query terms with a static curated code-concept synonym map.
#[derive(Debug, Default, Clone, Copy)]
pub struct SynonymExpander;

struct SynonymEntry {
    term: &'static str,
    synonyms: &'static [&'static str],
}

const SYNONYM_MAP: &[SynonymEntry] = &[
    SynonymEntry {
        term: "auth",
        synonyms: &[
            "auth",
            "authentication",
            "authorize",
            "authorization",
            "login",
            "session",
            "token",
            "jwt",
            "bearer",
            "oauth",
        ],
    },
    SynonymEntry {
        term: "db",
        synonyms: &[
            "db",
            "database",
            "query",
            "sql",
            "migration",
            "schema",
            "table",
        ],
    },
    SynonymEntry {
        term: "api",
        synonyms: &[
            "api",
            "endpoint",
            "route",
            "handler",
            "controller",
            "request",
            "response",
        ],
    },
    SynonymEntry {
        term: "config",
        synonyms: &[
            "config",
            "configuration",
            "settings",
            "options",
            "env",
            "environment",
        ],
    },
    SynonymEntry {
        term: "cache",
        synonyms: &["cache", "cached", "memoize", "memoization", "lru", "ttl"],
    },
    SynonymEntry {
        term: "queue",
        synonyms: &[
            "queue", "enqueue", "dequeue", "broker", "consumer", "producer",
        ],
    },
    SynonymEntry {
        term: "worker",
        synonyms: &["worker", "background", "task", "job", "executor", "thread"],
    },
    SynonymEntry {
        term: "job",
        synonyms: &["job", "task", "work", "scheduler", "cron", "background"],
    },
    SynonymEntry {
        term: "error",
        synonyms: &["error", "err", "exception", "failure", "fault", "panic"],
    },
    SynonymEntry {
        term: "log",
        synonyms: &["log", "logs", "logging", "trace", "debug", "warn", "error"],
    },
    SynonymEntry {
        term: "test",
        synonyms: &["test", "tests", "testing", "spec", "case", "assert"],
    },
    SynonymEntry {
        term: "mock",
        synonyms: &["mock", "stub", "fake", "double", "fixture", "spy"],
    },
    SynonymEntry {
        term: "fixture",
        synonyms: &["fixture", "sample", "seed", "setup", "testdata", "mock"],
    },
    SynonymEntry {
        term: "migration",
        synonyms: &[
            "migration",
            "migrate",
            "schema",
            "ddl",
            "upgrade",
            "version",
        ],
    },
    SynonymEntry {
        term: "schema",
        synonyms: &["schema", "model", "table", "field", "column", "contract"],
    },
    SynonymEntry {
        term: "model",
        synonyms: &["model", "entity", "record", "struct", "schema", "dto"],
    },
    SynonymEntry {
        term: "service",
        synonyms: &[
            "service",
            "manager",
            "client",
            "provider",
            "component",
            "facade",
        ],
    },
    SynonymEntry {
        term: "controller",
        synonyms: &["controller", "handler", "route", "endpoint", "action"],
    },
    SynonymEntry {
        term: "handler",
        synonyms: &["handler", "callback", "listener", "controller", "route"],
    },
    SynonymEntry {
        term: "route",
        synonyms: &["route", "router", "endpoint", "path", "url", "handler"],
    },
    SynonymEntry {
        term: "middleware",
        synonyms: &["middleware", "filter", "interceptor", "guard", "layer"],
    },
    SynonymEntry {
        term: "request",
        synonyms: &["request", "req", "input", "payload", "body", "params"],
    },
    SynonymEntry {
        term: "response",
        synonyms: &["response", "res", "reply", "output", "payload", "body"],
    },
    SynonymEntry {
        term: "token",
        synonyms: &["token", "jwt", "bearer", "credential", "secret", "auth"],
    },
    SynonymEntry {
        term: "session",
        synonyms: &["session", "cookie", "login", "state", "auth", "user"],
    },
    SynonymEntry {
        term: "user",
        synonyms: &["user", "account", "profile", "principal", "identity"],
    },
    SynonymEntry {
        term: "permission",
        synonyms: &["permission", "access", "acl", "policy", "authorization"],
    },
    SynonymEntry {
        term: "role",
        synonyms: &["role", "group", "permission", "grant", "scope"],
    },
    SynonymEntry {
        term: "validation",
        synonyms: &["validation", "validate", "validator", "check", "verify"],
    },
    SynonymEntry {
        term: "parser",
        synonyms: &["parser", "parse", "lexer", "tokenizer", "syntax"],
    },
    SynonymEntry {
        term: "serializer",
        synonyms: &["serializer", "serialize", "encoder", "marshal", "json"],
    },
    SynonymEntry {
        term: "deserializer",
        synonyms: &[
            "deserializer",
            "deserialize",
            "decoder",
            "unmarshal",
            "json",
        ],
    },
    SynonymEntry {
        term: "cli",
        synonyms: &["cli", "command", "terminal", "shell", "arg", "flag"],
    },
    SynonymEntry {
        term: "command",
        synonyms: &["command", "cmd", "cli", "subcommand", "action"],
    },
    SynonymEntry {
        term: "flag",
        synonyms: &["flag", "option", "arg", "argument", "switch"],
    },
    SynonymEntry {
        term: "env",
        synonyms: &["env", "environment", "variable", "dotenv", "config"],
    },
    SynonymEntry {
        term: "file",
        synonyms: &["file", "path", "source", "document", "blob"],
    },
    SynonymEntry {
        term: "path",
        synonyms: &["path", "filepath", "filename", "route", "location"],
    },
    SynonymEntry {
        term: "async",
        synonyms: &["async", "await", "future", "promise", "concurrent"],
    },
    SynonymEntry {
        term: "thread",
        synonyms: &["thread", "worker", "concurrency", "parallel", "task"],
    },
    SynonymEntry {
        term: "lock",
        synonyms: &["lock", "mutex", "rwlock", "guard", "synchronize"],
    },
    SynonymEntry {
        term: "retry",
        synonyms: &["retry", "retries", "backoff", "attempt", "resilience"],
    },
    SynonymEntry {
        term: "timeout",
        synonyms: &["timeout", "deadline", "duration", "expire", "cancel"],
    },
    SynonymEntry {
        term: "rate_limit",
        synonyms: &["rate_limit", "ratelimit", "throttle", "quota", "limit"],
    },
    SynonymEntry {
        term: "pagination",
        synonyms: &["pagination", "page", "cursor", "offset", "limit"],
    },
    SynonymEntry {
        term: "search",
        synonyms: &["search", "find", "query", "lookup", "retrieve"],
    },
    SynonymEntry {
        term: "index",
        synonyms: &["index", "indices", "catalog", "inverted", "lookup"],
    },
    SynonymEntry {
        term: "vector",
        synonyms: &["vector", "embedding", "dense", "ann", "similarity"],
    },
    SynonymEntry {
        term: "embedding",
        synonyms: &["embedding", "embed", "vector", "semantic", "encoder"],
    },
    SynonymEntry {
        term: "chunk",
        synonyms: &["chunk", "segment", "slice", "span", "section"],
    },
];

impl SynonymExpander {
    pub fn expand(query: &str) -> String {
        tokenize_query(query)
            .into_iter()
            .map(|token| match Self::synonyms_for(&token) {
                Some(synonyms) => format!("({})", synonyms.join(" OR ")),
                None => token,
            })
            .collect::<Vec<_>>()
            .join(" OR ")
    }

    pub fn synonyms_for(term: &str) -> Option<&'static [&'static str]> {
        let term = term.to_ascii_lowercase();
        SYNONYM_MAP
            .iter()
            .find(|entry| entry.term == term)
            .map(|entry| entry.synonyms)
    }

    pub fn entry_count() -> usize {
        SYNONYM_MAP.len()
    }
}

fn tokenize_query(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in query.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch.to_ascii_lowercase());
        } else {
            push_token(&mut tokens, &mut current);
        }
    }
    push_token(&mut tokens, &mut current);

    tokens
}

fn push_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn auth_expands_to_documented_or_group() {
        let expanded = SynonymExpander::expand("auth");

        assert!(expanded.starts_with('('), "{expanded}");
        for term in ["auth", "authentication", "login", "session", "token", "jwt"] {
            assert!(expanded.contains(term), "missing {term}: {expanded}");
        }
        assert!(expanded.contains(" OR "), "{expanded}");
    }

    #[test]
    fn unknown_token_is_unchanged() {
        assert_eq!(SynonymExpander::expand("frobnicate"), "frobnicate");
    }

    #[test]
    fn known_and_unknown_tokens_join_as_or_query() {
        let expanded = SynonymExpander::expand("auth frobnicate");

        assert!(expanded.contains("(auth OR authentication"));
        assert!(expanded.ends_with(" OR frobnicate"), "{expanded}");
    }

    #[test]
    fn query_is_lowercased_and_tokenized() {
        let expanded = SynonymExpander::expand("Auth,DB!");

        assert!(expanded.contains("(auth OR authentication"));
        assert!(expanded.contains(" OR (db OR database"));
    }

    #[test]
    fn map_has_curated_code_concept_count() {
        assert_eq!(SynonymExpander::entry_count(), 50);
    }

    #[test]
    fn every_entry_includes_its_own_term_first() {
        for entry in SYNONYM_MAP {
            assert_eq!(entry.synonyms.first(), Some(&entry.term), "{}", entry.term);
        }
    }
}
