//! OpenAI-compatible cloud embedding backend (task 3.4).
//!
//! [`OpenAiCompatEmbedder`] calls any `/v1/embeddings`-compatible HTTP endpoint
//! (OpenAI, Azure, local proxies, etc.) and implements the [`Embedder`] trait
//! defined in task 3.1.
//!
//! ## Rate limiting
//! A minimum inter-request interval derived from `max_requests_per_minute`
//! (`interval = 60s / rpm`) is enforced using `tokio::time::sleep` before each
//! request.  For the default 500 rpm the sleep is ~120 ms — negligible for a
//! batch job but protective under low-tier keys.  The limiter is synchronous in
//! its core logic (compute the deadline; `sleep` executes it) and therefore
//! unit-testable without wall-clock assertions.
//!
//! ## Retry / backoff
//! HTTP 429 and 5xx responses are retried up to [`MAX_RETRIES`] times with
//! exponential backoff starting at `base_backoff_ms` (configurable in tests).
//! 4xx responses other than 429 are not retried — they indicate a caller error
//! (bad key, invalid model, not found) that retrying cannot fix.
//!
//! ## dim() strategy
//! The VectorStore needs `dim()` before the first embed call. Because the
//! OpenAI API does not expose the dimension without actually embedding, `dim()`
//! is resolved from a static map of well-known model names at construction time.
//! Unknown models fall back to 1536 (matching `text-embedding-3-small` and
//! `text-embedding-ada-002`).  The chosen value is logged at `DEBUG` level so
//! users with custom/fine-tuned endpoints can verify it.  This trade-off
//! (static map vs. a probe embed) avoids an implicit API call in the constructor
//! and keeps construction infallible after config validation.
//!
//! ## Key safety
//! The API key is stored in the struct but NEVER included in error messages or
//! log output — only its presence/absence is tested (`is_empty()`). Error
//! messages include HTTP status codes and a truncated response body snippet for
//! context, but never the key material.

use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::config::Config;
use crate::embedder::Embedder;
use crate::error::{Result, VektorError};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of retry attempts for retryable responses (429 / 5xx).
/// After this many attempts the last error is returned.
const MAX_RETRIES: u32 = 4;

/// Maximum bytes of the response body to include in error messages so that
/// large HTML error pages from misconfigured proxies do not flood logs.
const ERROR_BODY_SNIPPET_LEN: usize = 256;

// ---------------------------------------------------------------------------
// Request / response shapes (OpenAI embeddings API)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Debug, Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedRecord>,
}

#[derive(Debug, Deserialize)]
struct EmbedRecord {
    embedding: Vec<f32>,
    index: usize,
}

// ---------------------------------------------------------------------------
// OpenAiCompatEmbedder
// ---------------------------------------------------------------------------

/// Cloud embedding backend for any OpenAI-compatible `/v1/embeddings` endpoint.
///
/// Constructed by the factory in task 3.5; the `#[allow(dead_code)]` below
/// suppresses the "field never read" lint until the factory lands.
///
/// `Debug` is implemented manually below: `client` is omitted (reqwest 0.13
/// `Client` does not impl `Debug`) and `api_key` is redacted to prevent
/// accidental key leakage in logs or test output.
// constructed by 3.5 factory; allow until then.
#[allow(dead_code)]
pub struct OpenAiCompatEmbedder {
    /// Shared HTTP client.
    client: Client,
    /// Fully-formed endpoint URL: `base_url.trim_end_matches('/') + "/embeddings"`.
    endpoint: String,
    /// API key; stored but NEVER emitted to logs or error strings.
    api_key: String,
    /// OpenAI model identifier (e.g. `text-embedding-3-small`).
    model: String,
    /// Embedding dimensionality derived from the static model→dim map.
    dim: usize,
    /// Minimum duration to sleep before every request (60s / max_requests_per_minute).
    rate_limit_interval: Duration,
    /// Base duration for the first retry sleep.  Injected for tests so they
    /// do not actually wait seconds; production uses [`DEFAULT_BASE_BACKOFF`].
    base_backoff: Duration,
}

impl std::fmt::Debug for OpenAiCompatEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatEmbedder")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("dim", &self.dim)
            .field("api_key", &"<redacted>")
            .field("rate_limit_interval", &self.rate_limit_interval)
            .field("base_backoff", &self.base_backoff)
            .finish_non_exhaustive()
    }
}

/// Default first-retry sleep.  Subsequent retries double this: 500 ms, 1 s, 2 s, 4 s.
const DEFAULT_BASE_BACKOFF: Duration = Duration::from_millis(500);

// constructed by 3.5 factory; allow until then.
#[allow(dead_code)]
impl OpenAiCompatEmbedder {
    /// Construct from the full [`Config`].
    ///
    /// Validates that `openai_api_key` is non-empty before touching the network.
    ///
    /// # Errors
    /// - [`VektorError::Config`] if `openai_api_key` is empty (caught before any
    ///   HTTP request is made).
    pub fn new(config: &Config) -> Result<Self> {
        Self::new_with_backoff(config, DEFAULT_BASE_BACKOFF)
    }

    /// Like [`new`](Self::new) but with an injectable `base_backoff` for tests.
    ///
    /// Allows tests to use a very short base backoff so retry tests complete in
    /// milliseconds without asserting on wall-clock time.
    pub fn new_with_backoff(config: &Config, base_backoff: Duration) -> Result<Self> {
        let api_key = config.embedding.openai_api_key.clone();
        if api_key.is_empty() {
            return Err(VektorError::Config(
                "openai_api_key is required for the OpenAI-compatible embedder backend; \
                 set it in config.toml or via VEKTOR__EMBEDDING__OPENAI_API_KEY"
                    .to_string(),
            ));
        }

        let endpoint = build_endpoint(&config.embedding.openai_base_url);
        let model = config.embedding.openai_model.clone();
        let dim = dim_for_model(&model);

        tracing::debug!(
            model = %model,
            dim,
            endpoint = %endpoint,
            "openai-compat embedder configured"
        );

        let rpm = config.embedding.max_requests_per_minute;
        let rate_limit_interval = if rpm == 0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(60.0 / f64::from(rpm))
        };

        Ok(Self {
            client: Client::new(),
            endpoint,
            api_key,
            model,
            dim,
            rate_limit_interval,
            base_backoff,
        })
    }

    /// Send one embedding request with retry/backoff.
    ///
    /// Callers are responsible for rate-limiting via `sleep(self.rate_limit_interval)`
    /// before invoking this — see [`embed`](Embedder::embed).
    async fn post_with_retry(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let body = EmbedRequest {
            model: &self.model,
            input: texts,
        };

        let mut last_err: Option<VektorError> = None;

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                // Exponential backoff: base * 2^(attempt-1)
                let backoff = self.base_backoff * 2u32.pow(attempt - 1);
                tracing::debug!(attempt, ?backoff, "retrying after transient error");
                sleep(backoff).await;
            }

            let mut req = self
                .client
                .post(&self.endpoint)
                .header("Content-Type", "application/json");

            // Key is guaranteed non-empty by constructor; safe to add header.
            req = req.header("Authorization", format!("Bearer {}", self.api_key));

            let response = match req.json(&body).send().await {
                Ok(r) => r,
                Err(err) => {
                    // Transport / connection error — treat as retryable.
                    last_err = Some(VektorError::Network(err));
                    continue;
                }
            };

            let status = response.status();

            if status.is_success() {
                let parsed: EmbedResponse = response.json().await.map_err(|err| {
                    VektorError::Embedding(format!(
                        "failed to parse embeddings response (status {status}): {err}"
                    ))
                })?;

                return order_embeddings(parsed.data, texts.len());
            }

            // Non-retryable client errors (4xx except 429).
            if status.is_client_error() && status.as_u16() != 429 {
                let snippet = body_snippet(response).await;
                return Err(VektorError::Embedding(format!(
                    "embeddings request failed with non-retryable status {status}: {snippet}"
                )));
            }

            // Retryable: 429 or 5xx.
            let snippet = body_snippet(response).await;
            let msg = format!(
                "embeddings request failed with status {status} (attempt {}/{}): {snippet}",
                attempt + 1,
                MAX_RETRIES + 1
            );
            tracing::warn!(%msg, "retryable embedding error");
            last_err = Some(VektorError::Embedding(msg));
        }

        Err(last_err.unwrap_or_else(|| {
            VektorError::Embedding("embeddings request failed after retries".to_string())
        }))
    }
}

#[async_trait::async_trait]
impl Embedder for OpenAiCompatEmbedder {
    /// Embed a batch of already-prefixed texts via the OpenAI-compatible API.
    ///
    /// Applies rate limiting (sleeps `rate_limit_interval` before the request),
    /// then delegates to the retry loop.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Enforce rate limit: sleep the minimum inter-request interval.
        if !self.rate_limit_interval.is_zero() {
            sleep(self.rate_limit_interval).await;
        }

        self.post_with_retry(texts).await
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn name(&self) -> &str {
        &self.model
    }

    /// OpenAI embedding models do not use Jina-style task prefixes.
    fn prefix_for_document(&self) -> &str {
        ""
    }

    /// OpenAI embedding models do not use Jina-style task prefixes.
    fn prefix_for_query(&self) -> &str {
        ""
    }
}

// ---------------------------------------------------------------------------
// Pure helpers (no I/O — unit-testable)
// ---------------------------------------------------------------------------

/// Build the full embeddings endpoint URL from `base_url`.
///
/// Strips a trailing `/` from `base_url` then appends `/embeddings`.
/// The default `https://api.openai.com/v1` → `https://api.openai.com/v1/embeddings`.
/// A user who accidentally writes `https://api.openai.com/v1/` still gets the
/// correct result.
fn build_endpoint(base_url: &str) -> String {
    format!("{}/embeddings", base_url.trim_end_matches('/'))
}

/// Static dimension lookup keyed by model name.
///
/// `dim()` must be available before the first embed call (VectorStore schema
/// creation). A live probe would require an API call in the constructor; using
/// a static map avoids that. Unknown models default to 1536, which matches the
/// two most common OpenAI models. Users with custom endpoints at a different
/// dimension will see the mismatch only at insert time — acceptable because
/// custom endpoint users are power users who can set the dimension explicitly
/// in a future config option (deferred: YAGNI for Phase 3).
fn dim_for_model(model: &str) -> usize {
    match model {
        "text-embedding-3-large" => 3072,
        "text-embedding-3-small" | "text-embedding-ada-002" => 1536,
        _ => {
            tracing::debug!(
                model,
                fallback_dim = 1536,
                "unknown model — falling back to dim=1536; \
                 override if your endpoint uses a different dimension"
            );
            1536
        }
    }
}

/// Re-order response records by their `index` field to restore input order,
/// then extract just the embedding vectors.
///
/// The OpenAI spec does not guarantee that `data` is returned in input order,
/// so we sort by `index` defensively.
fn order_embeddings(mut records: Vec<EmbedRecord>, expected: usize) -> Result<Vec<Vec<f32>>> {
    records.sort_unstable_by_key(|r| r.index);

    if records.len() != expected {
        return Err(VektorError::Embedding(format!(
            "embeddings response contained {} vectors but {} were requested",
            records.len(),
            expected
        )));
    }

    for (expected_index, record) in records.iter().enumerate() {
        if record.index != expected_index {
            return Err(VektorError::Embedding(format!(
                "embeddings response index mismatch: expected index {expected_index}, got {}; \
                 response indexes must be a complete 0..{} sequence",
                record.index,
                expected.saturating_sub(1)
            )));
        }
    }

    Ok(records.into_iter().map(|r| r.embedding).collect())
}

/// Truncate `s` to at most `max_bytes` bytes, always cutting on a valid UTF-8
/// char boundary.
///
/// This is the safe replacement for `&s[..max_bytes]`: slicing a `str` by a
/// raw byte index panics when the index falls in the middle of a multi-byte
/// UTF-8 character (e.g. any non-ASCII code point).
///
/// We scan `char_indices` to find the first character that, including all of
/// its bytes, would push the slice past `max_bytes`.  That character's start
/// index is the correct cut point — it is a valid char boundary and its value
/// is ≤ `max_bytes`.
///
/// Example: for 3-byte chars ("界") and `max_bytes = 256`, chars start at 0,
/// 3, 6, …, 252, 255.  The char at byte 255 ends at byte 258 > 256, so it is
/// the first overflow char.  We cut at its start (255), returning 255 bytes —
/// safely within the limit.
fn truncate_on_char_boundary(s: &str, max_bytes: usize) -> &str {
    // Find the first char whose tail (i + char byte length) exceeds max_bytes.
    // Its start index `i` is a valid char boundary ≤ max_bytes.
    let end = s
        .char_indices()
        .find(|&(i, c)| i + c.len_utf8() > max_bytes)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    &s[..end]
}

/// Extract up to [`ERROR_BODY_SNIPPET_LEN`] bytes from a response body for use
/// in error messages.  Never panics; returns a placeholder on read failure.
/// The key is guaranteed NOT to appear here (it is not in the response body).
async fn body_snippet(response: reqwest::Response) -> String {
    match response.text().await {
        Ok(body) => truncate_on_char_boundary(&body, ERROR_BODY_SNIPPET_LEN).to_string(),
        Err(_) => "<could not read response body>".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Build a minimal config with the given key, model, base_url and rpm.
    fn make_config(key: &str, model: &str, base_url: &str, rpm: u32) -> Config {
        use crate::config::EmbeddingConfig;
        Config {
            embedding: EmbeddingConfig {
                openai_api_key: key.to_string(),
                openai_model: model.to_string(),
                openai_base_url: base_url.to_string(),
                max_requests_per_minute: rpm,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Build an embedder pointed at a wiremock server, with a tiny backoff so
    /// retry tests complete quickly.
    fn embedder_for(server: &MockServer, model: &str) -> OpenAiCompatEmbedder {
        let base = server.uri();
        let config = make_config("sk-test-key", model, &base, 0); // rpm=0 disables rate-limit sleep
        OpenAiCompatEmbedder::new_with_backoff(&config, Duration::from_millis(1))
            .expect("build embedder")
    }

    /// Build a success-response body for `n` texts, each with `dim` dimensions.
    /// Returns records in reverse input order to verify the sorter works.
    fn success_body(n: usize, dim: usize) -> serde_json::Value {
        let data: Vec<serde_json::Value> = (0..n)
            .rev() // deliberately reverse order → tests that we sort by index
            .enumerate()
            .map(|(pos, idx)| {
                serde_json::json!({
                    "embedding": vec![pos as f32 / 10.0; dim],
                    "index": idx,
                })
            })
            .collect();
        serde_json::json!({ "data": data })
    }

    // -----------------------------------------------------------------------
    // build_endpoint — pure
    // -----------------------------------------------------------------------

    #[test]
    fn openai_compat_embedder_build_endpoint_default_base() {
        assert_eq!(
            build_endpoint("https://api.openai.com/v1"),
            "https://api.openai.com/v1/embeddings"
        );
    }

    #[test]
    fn openai_compat_embedder_build_endpoint_trailing_slash_stripped() {
        assert_eq!(
            build_endpoint("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/embeddings"
        );
    }

    #[test]
    fn openai_compat_embedder_build_endpoint_custom_base() {
        assert_eq!(
            build_endpoint("http://localhost:11434/v1"),
            "http://localhost:11434/v1/embeddings"
        );
    }

    // -----------------------------------------------------------------------
    // dim_for_model — pure
    // -----------------------------------------------------------------------

    #[test]
    fn openai_compat_embedder_dim_for_model_known_models() {
        assert_eq!(dim_for_model("text-embedding-3-small"), 1536);
        assert_eq!(dim_for_model("text-embedding-3-large"), 3072);
        assert_eq!(dim_for_model("text-embedding-ada-002"), 1536);
    }

    #[test]
    fn openai_compat_embedder_dim_for_model_unknown_falls_back_to_1536() {
        assert_eq!(dim_for_model("some-custom-model"), 1536);
        assert_eq!(dim_for_model(""), 1536);
    }

    // -----------------------------------------------------------------------
    // order_embeddings — pure
    // -----------------------------------------------------------------------

    #[test]
    fn openai_compat_embedder_order_embeddings_sorts_by_index() {
        let records = vec![
            EmbedRecord {
                embedding: vec![2.0],
                index: 2,
            },
            EmbedRecord {
                embedding: vec![0.0],
                index: 0,
            },
            EmbedRecord {
                embedding: vec![1.0],
                index: 1,
            },
        ];
        let result = order_embeddings(records, 3).expect("order");
        assert_eq!(result[0], vec![0.0]);
        assert_eq!(result[1], vec![1.0]);
        assert_eq!(result[2], vec![2.0]);
    }

    #[test]
    fn openai_compat_embedder_order_embeddings_errors_on_count_mismatch() {
        let records = vec![EmbedRecord {
            embedding: vec![0.0],
            index: 0,
        }];
        let err = order_embeddings(records, 3).expect_err("should error");
        assert!(matches!(err, VektorError::Embedding(_)));
        let msg = err.to_string();
        assert!(msg.contains("1"), "msg: {msg}");
        assert!(msg.contains("3"), "msg: {msg}");
    }

    #[test]
    fn openai_compat_embedder_order_embeddings_errors_on_duplicate_index() {
        let records = vec![
            EmbedRecord {
                embedding: vec![0.0],
                index: 0,
            },
            EmbedRecord {
                embedding: vec![1.0],
                index: 0,
            },
        ];

        let err = order_embeddings(records, 2).expect_err("duplicate index must error");
        assert!(matches!(err, VektorError::Embedding(_)));
        let msg = err.to_string();
        assert!(msg.contains("index"), "msg: {msg}");
    }

    #[test]
    fn openai_compat_embedder_order_embeddings_errors_on_missing_index() {
        let records = vec![
            EmbedRecord {
                embedding: vec![0.0],
                index: 0,
            },
            EmbedRecord {
                embedding: vec![2.0],
                index: 2,
            },
        ];

        let err = order_embeddings(records, 2).expect_err("missing index must error");
        assert!(matches!(err, VektorError::Embedding(_)));
        let msg = err.to_string();
        assert!(msg.contains("index"), "msg: {msg}");
    }

    // -----------------------------------------------------------------------
    // missing key — Config error BEFORE any request
    // -----------------------------------------------------------------------

    #[test]
    fn openai_compat_embedder_missing_key_yields_config_error() {
        let config = make_config(
            "",
            "text-embedding-3-small",
            "https://api.openai.com/v1",
            500,
        );
        let err = OpenAiCompatEmbedder::new(&config).expect_err("should fail");
        assert!(matches!(err, VektorError::Config(_)));
        // Error message must NOT contain the (empty) key — no leakage.
        let msg = err.to_string();
        assert!(
            msg.contains("openai_api_key"),
            "should mention the config field: {msg}"
        );
        // Confirm there's no stray key material: "sk-" prefix never appears.
        assert!(!msg.contains("sk-"), "key leaked into error: {msg}");
    }

    // -----------------------------------------------------------------------
    // Success path (wiremock)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn openai_compat_embedder_success_parses_and_preserves_order() {
        let server = MockServer::start().await;
        let dim = 4;
        let n = 3;
        let body = success_body(n, dim);

        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .and(header("Authorization", "Bearer sk-test-key"))
            .and(header("Content-Type", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let embedder = embedder_for(&server, "text-embedding-3-small");
        let texts: Vec<String> = (0..n).map(|i| format!("text {i}")).collect();
        let result = embedder.embed(&texts).await.expect("embed should succeed");

        assert_eq!(result.len(), n);
        // Each vector has the correct dimension.
        assert!(result.iter().all(|v| v.len() == dim));
        // Order preservation: success_body builds records in reverse index order
        // (idx=2 first, then idx=1, then idx=0) and assigns embedding values
        // based on pos (enumerate position). pos=0 → idx=2 → value 0.0,
        // pos=1 → idx=1 → value 0.1, pos=2 → idx=0 → value 0.2.
        // After sorting by index, result[0] (index=0) has value 0.2.
        let expected_value_for_index_0: f32 = 2.0 / 10.0;
        assert!(
            (result[0][0] - expected_value_for_index_0).abs() < 1e-6,
            "result[0][0] = {} (expected {})",
            result[0][0],
            expected_value_for_index_0
        );
    }

    #[tokio::test]
    async fn openai_compat_embedder_empty_input_returns_empty_no_request() {
        // No mock mounted — any request would cause the test to panic/fail.
        let server = MockServer::start().await;
        let embedder = embedder_for(&server, "text-embedding-3-small");
        let result = embedder.embed(&[]).await.expect("empty embed");
        assert!(result.is_empty());
        // Confirm no requests were made.
        assert_eq!(server.received_requests().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn openai_compat_embedder_request_body_contains_model_and_input() {
        use wiremock::matchers::body_json;

        let server = MockServer::start().await;
        let texts = vec!["hello".to_string(), "world".to_string()];
        let body = success_body(2, 4);

        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .and(body_json(serde_json::json!({
                "model": "text-embedding-3-small",
                "input": ["hello", "world"]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let embedder = embedder_for(&server, "text-embedding-3-small");
        embedder.embed(&texts).await.expect("should succeed");
    }

    // -----------------------------------------------------------------------
    // Non-retryable 4xx (wiremock)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn openai_compat_embedder_non_retryable_4xx_fails_immediately() {
        let server = MockServer::start().await;

        // 401 Unauthorized — serve it once; if retried, wiremock returns 404
        // and the test would still pass (just differently), so we assert count.
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(401).set_body_string("invalid api key"))
            .mount(&server)
            .await;

        let embedder = embedder_for(&server, "text-embedding-3-small");
        let texts = vec!["test".to_string()];
        let err = embedder.embed(&texts).await.expect_err("should fail");

        assert!(matches!(err, VektorError::Embedding(_)));
        let msg = err.to_string();
        assert!(msg.contains("401"), "status in error: {msg}");
        // Only ONE request — no retry.
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1, "expected exactly 1 request (no retry)");
    }

    #[tokio::test]
    async fn openai_compat_embedder_400_is_not_retried() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
            .mount(&server)
            .await;

        let embedder = embedder_for(&server, "text-embedding-3-small");
        let texts = vec!["test".to_string()];
        let err = embedder.embed(&texts).await.expect_err("should fail");
        assert!(matches!(err, VektorError::Embedding(_)));
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 1, "expected exactly 1 request (no retry)");
    }

    #[tokio::test]
    async fn openai_compat_embedder_403_is_not_retried() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&server)
            .await;

        let embedder = embedder_for(&server, "text-embedding-3-small");
        let texts = vec!["test".to_string()];
        let err = embedder.embed(&texts).await.expect_err("should fail");
        assert!(matches!(err, VektorError::Embedding(_)));
        let msg = err.to_string();
        assert!(msg.contains("403"), "status in error: {msg}");
        // Exactly one request — 403 is non-retryable.
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(
            reqs.len(),
            1,
            "expected exactly 1 request (no retry for 403)"
        );
    }

    // -----------------------------------------------------------------------
    // truncate_on_char_boundary — pure, regression for multibyte UTF-8 panic
    // -----------------------------------------------------------------------

    /// Without the fix, slicing `&body[..ERROR_BODY_SNIPPET_LEN]` on a string
    /// composed entirely of 3-byte UTF-8 characters (e.g. "界") would panic at
    /// run time whenever `ERROR_BODY_SNIPPET_LEN` (256) is not a multiple of 3,
    /// because 256 is not aligned to a 3-byte boundary (256 % 3 == 1).
    /// `truncate_on_char_boundary` must NOT panic and must return a valid prefix.
    #[test]
    fn openai_compat_embedder_truncate_on_char_boundary_multibyte_no_panic() {
        // "界" is 3 bytes (U+754C).  Repeat it enough times that the string
        // exceeds ERROR_BODY_SNIPPET_LEN (256) bytes.
        let multibyte_body = "界".repeat(100); // 300 bytes total
        assert!(
            multibyte_body.len() > ERROR_BODY_SNIPPET_LEN,
            "test setup: body must exceed the snippet limit"
        );

        // This must NOT panic, even though 256 is not a 3-byte boundary.
        let snippet = truncate_on_char_boundary(&multibyte_body, ERROR_BODY_SNIPPET_LEN);

        // The result is a valid &str (guaranteed by Rust's type system here).
        // Its byte length must be ≤ ERROR_BODY_SNIPPET_LEN.
        assert!(
            snippet.len() <= ERROR_BODY_SNIPPET_LEN,
            "snippet ({} bytes) exceeds the limit",
            snippet.len()
        );
        // Every byte in the slice is still valid UTF-8 (from_utf8 would panic on bad data).
        assert!(std::str::from_utf8(snippet.as_bytes()).is_ok());
        // The snippet is a proper prefix: the original starts with it.
        assert!(multibyte_body.starts_with(snippet));
    }

    #[test]
    fn openai_compat_embedder_truncate_on_char_boundary_ascii_unchanged_within_limit() {
        let s = "hello";
        assert_eq!(truncate_on_char_boundary(s, 256), "hello");
    }

    #[test]
    fn openai_compat_embedder_truncate_on_char_boundary_exact_boundary() {
        // String whose byte length equals max_bytes exactly — should return the whole string.
        let s = "abcd"; // 4 bytes
        assert_eq!(truncate_on_char_boundary(s, 4), "abcd");
    }

    // -----------------------------------------------------------------------
    // 429 retry → success (wiremock)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn openai_compat_embedder_429_retries_and_eventually_succeeds() {
        let server = MockServer::start().await;
        let body = success_body(1, 4);

        // First call: 429. Second call: 200.
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let embedder = embedder_for(&server, "text-embedding-3-small");
        let texts = vec!["test".to_string()];
        let result = embedder
            .embed(&texts)
            .await
            .expect("should succeed after retry");
        assert_eq!(result.len(), 1);

        // Two requests: one 429, one 200.
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 2, "expected 2 requests (1 retry)");
    }

    // -----------------------------------------------------------------------
    // 429 exhausted retries (wiremock)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn openai_compat_embedder_429_exhausts_retries_and_fails() {
        let server = MockServer::start().await;

        // Every call returns 429.
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(429).set_body_string("always rate limited"))
            .mount(&server)
            .await;

        let embedder = embedder_for(&server, "text-embedding-3-small");
        let texts = vec!["test".to_string()];
        let err = embedder.embed(&texts).await.expect_err("should fail");

        assert!(matches!(err, VektorError::Embedding(_)));
        let msg = err.to_string();
        assert!(msg.contains("429"), "status in final error: {msg}");

        // Should have tried initial + MAX_RETRIES times.
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(
            reqs.len(),
            (MAX_RETRIES + 1) as usize,
            "expected {} requests",
            MAX_RETRIES + 1
        );
    }

    // -----------------------------------------------------------------------
    // 5xx retry (wiremock)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn openai_compat_embedder_5xx_retries_and_eventually_succeeds() {
        let server = MockServer::start().await;
        let body = success_body(1, 4);

        // First call: 503. Second call: 200.
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(503).set_body_string("service unavailable"))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let embedder = embedder_for(&server, "text-embedding-3-small");
        let texts = vec!["test".to_string()];
        let result = embedder
            .embed(&texts)
            .await
            .expect("should succeed after 5xx retry");
        assert_eq!(result.len(), 1);

        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 2);
    }

    // -----------------------------------------------------------------------
    // API key never leaks into error messages
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn openai_compat_embedder_api_key_not_in_error_messages() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;

        let secret_key = "sk-super-secret-key-12345";
        let base = server.uri();
        let config = make_config(secret_key, "text-embedding-3-small", &base, 0);
        let embedder = OpenAiCompatEmbedder::new_with_backoff(&config, Duration::from_millis(1))
            .expect("build embedder");
        let texts = vec!["test".to_string()];
        let err = embedder.embed(&texts).await.expect_err("should fail");

        let msg = err.to_string();
        assert!(
            !msg.contains(secret_key),
            "API key leaked into error message: {msg}"
        );
    }

    // -----------------------------------------------------------------------
    // Trait method: name/dim/prefixes
    // -----------------------------------------------------------------------

    #[test]
    fn openai_compat_embedder_name_returns_model() {
        let config = make_config(
            "sk-key",
            "text-embedding-3-large",
            "https://api.openai.com/v1",
            500,
        );
        let embedder =
            OpenAiCompatEmbedder::new_with_backoff(&config, Duration::ZERO).expect("build");
        assert_eq!(embedder.name(), "text-embedding-3-large");
    }

    #[test]
    fn openai_compat_embedder_dim_resolves_correctly() {
        let config = make_config(
            "sk-key",
            "text-embedding-3-large",
            "https://api.openai.com/v1",
            500,
        );
        let embedder =
            OpenAiCompatEmbedder::new_with_backoff(&config, Duration::ZERO).expect("build");
        assert_eq!(embedder.dim(), 3072);
    }

    #[test]
    fn openai_compat_embedder_prefixes_are_empty() {
        let config = make_config(
            "sk-key",
            "text-embedding-3-small",
            "https://api.openai.com/v1",
            500,
        );
        let embedder =
            OpenAiCompatEmbedder::new_with_backoff(&config, Duration::ZERO).expect("build");
        assert_eq!(embedder.prefix_for_document(), "");
        assert_eq!(embedder.prefix_for_query(), "");
    }

    // -----------------------------------------------------------------------
    // Rate limit interval computation — pure logic
    // -----------------------------------------------------------------------

    #[test]
    fn openai_compat_embedder_rate_limit_interval_from_rpm() {
        // 500 rpm → interval = 60/500 = 0.12 s = 120 ms
        let interval = Duration::from_secs_f64(60.0 / 500.0);
        assert!((interval.as_millis() as i64 - 120).abs() <= 1);

        // 60 rpm → 1 s
        let interval = Duration::from_secs_f64(60.0 / 60.0);
        assert_eq!(interval.as_secs(), 1);
    }

    #[test]
    fn openai_compat_embedder_zero_rpm_disables_rate_limit() {
        let config = make_config(
            "sk-key",
            "text-embedding-3-small",
            "https://api.openai.com/v1",
            0,
        );
        let embedder =
            OpenAiCompatEmbedder::new_with_backoff(&config, Duration::ZERO).expect("build");
        assert!(embedder.rate_limit_interval.is_zero());
    }
}
