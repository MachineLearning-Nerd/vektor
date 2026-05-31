use thiserror::Error;

pub type Result<T> = std::result::Result<T, VektorError>;

#[derive(Debug, Error)]
pub enum VektorError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("not implemented: {0}")]
    NotImplemented(&'static str),

    #[error("parse error: {0}")]
    Parse(String),

    /// SQLite / HashStore state errors. Kept distinct from Storage (vector DB).
    #[error("state error: {0}")]
    State(String),

    #[error("MCP protocol error: {0}")]
    Mcp(String),

    /// ONNX Runtime / tokenizer failures.
    /// `ort::Error` and `tokenizers` errors may not satisfy `Send + Sync + 'static`
    /// required by `#[from]`; callers use `.map_err(|e| VektorError::Embedding(e.to_string()))`.
    /// `#[allow(dead_code)]` until task 3.2/3.4 wires real construction.
    #[allow(dead_code)]
    #[error("embedding error: {0}")]
    Embedding(String),

    /// LanceDB / Arrow vector-store failures.
    /// `lancedb::Error` may not satisfy `Send + Sync + 'static` required by `#[from]`;
    /// callers use `.map_err(|e| VektorError::Storage(e.to_string()))`.
    /// `#[allow(dead_code)]` until task 3.6 wires real construction.
    #[allow(dead_code)]
    #[error("storage error: {0}")]
    Storage(String),

    /// Reqwest HTTP / network failures (cloud embedding APIs, model downloads).
    /// `reqwest::Error` is `Send + Sync + 'static`, so `#[from]` compiles cleanly.
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// Model-artifact download failures that are NOT transport-level `reqwest::Error`s
    /// — e.g. a server returning an HTTP error status (404/5xx) for a model file.
    /// Distinct from [`Self::Network`] (which wraps `reqwest::Error` for connection/timeout
    /// failures) and from [`Self::Config`] (which is for configuration problems). Used by
    /// `vektor models download`.
    #[error("download error: {0}")]
    Download(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_io_error() {
        let error: VektorError =
            std::io::Error::new(std::io::ErrorKind::NotFound, "missing").into();

        assert!(error.to_string().contains("IO error"));
    }

    #[test]
    fn display_config_error() {
        let error = VektorError::Config("bad value".into());

        assert_eq!(error.to_string(), "config error: bad value");
    }

    #[test]
    fn display_mcp_error() {
        let error = VektorError::Mcp("invalid request".into());

        assert_eq!(error.to_string(), "MCP protocol error: invalid request");
    }

    #[test]
    fn result_alias_uses_vektor_error() {
        let result: Result<()> = Err(VektorError::NotImplemented("test"));

        assert!(matches!(result, Err(VektorError::NotImplemented("test"))));
    }

    #[test]
    fn display_embedding_error() {
        let error = VektorError::Embedding("ort session failed".into());

        assert_eq!(error.to_string(), "embedding error: ort session failed");
        assert!(matches!(error, VektorError::Embedding(_)));
    }

    #[test]
    fn display_storage_error() {
        let error = VektorError::Storage("table not found".into());

        assert_eq!(error.to_string(), "storage error: table not found");
        assert!(matches!(error, VektorError::Storage(_)));
    }

    #[test]
    fn display_network_error() {
        // Construct a reqwest::Error via a URL-parse failure (no I/O or async needed).
        // reqwest::Url::parse returns a url::ParseError; reqwest::Error wraps it via
        // reqwest::Client::get → build → the builder rejects invalid URLs synchronously.
        // Using the public builder API: `reqwest::Client::new().get("://bad").build()`.
        let reqwest_err = reqwest::Client::new()
            .get("://bad-url")
            .build()
            .unwrap_err();
        let error: VektorError = reqwest_err.into();

        assert!(
            error.to_string().starts_with("network error:"),
            "expected 'network error: ...' but got: {error}"
        );
        assert!(matches!(error, VektorError::Network(_)));
    }

    #[test]
    fn display_download_error() {
        let error = VektorError::Download("HTTP 404 (url=...)".into());

        assert_eq!(error.to_string(), "download error: HTTP 404 (url=...)");
        assert!(matches!(error, VektorError::Download(_)));
    }
}
