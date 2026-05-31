//! Model artifact downloader — task 3.11 (`vektor models download`).
//!
//! Downloads the required ONNX model artifacts (tokenizer + ONNX graph) from
//! HuggingFace into the local model cache directory used by
//! [`crate::embedder::onnx::OnnxEmbedder`]. The download path scheme matches
//! task 3.2 exactly: `<data_dir>/models/<safe-model-name>/`, where `safe-model-name`
//! replaces `/` with `--`.
//!
//! ## Design decisions
//!
//! ### reqwest feature
//! We consume the response body via `Response::chunk().await` which works with
//! the `json`-only feature already in `Cargo.toml`. No new reqwest feature flag
//! was added (YAGNI — `bytes_stream()` is not needed here).
//!
//! ### Resume strategy
//! When a `.part` file already exists we send `Range: bytes=<existing_len>-`.
//! - HTTP `206 Partial Content` → append response body to the existing `.part`.
//! - HTTP `200 OK` (server does not support range) → delete the partial and restart.
//! - Any other status → return a clear error naming the failing URL.
//!
//! ### Atomic delivery
//! Body is streamed into `<file>.part`, then `fs::rename` to the final path
//! atomically after the last byte arrives. A failed download leaves only the
//! `.part` (resumable next run); the final path is never half-written.
//!
//! ### Idempotency
//! If the final file already exists (any non-zero length), it is treated as
//! complete and the download is skipped. A zero-byte file (truncation artifact)
//! is re-downloaded.
//!
//! ### Progress
//! `indicatif` progress bar written to **stderr** so stdout stays script-friendly.
//!
//! ### Base-URL override (test seam)
//! The HuggingFace base URL is provided by the caller (defaults to
//! `https://huggingface.co`). Tests point this at a local `wiremock` server
//! instead, so no network is required in CI.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use indicatif::{ProgressBar, ProgressStyle};
use reqwest::{Client, StatusCode, header};

use crate::{
    config::Config,
    error::{Result, VektorError},
    state::expand_data_dir,
};

// ---------------------------------------------------------------------------
// Static model table
// ---------------------------------------------------------------------------

/// One artifact to download: `(remote_path, local_relative_path)`.
/// `remote_path` is appended to `https://huggingface.co/<repo>/resolve/main/`.
/// `local_relative_path` is relative to the model cache dir
/// (`<data_dir>/models/<safe-name>/`).
pub struct Artifact {
    pub remote_path: &'static str,
    pub local_path: &'static str,
}

/// Static descriptor for a downloadable ONNX model.
pub struct ModelSpec {
    /// HuggingFace repo id (e.g. `jinaai/jina-embeddings-v2-base-code`).
    pub model_name: &'static str,
    /// Embedding dimensionality — informational, used in progress messages.
    pub dim: usize,
    /// HuggingFace repo slug (e.g. `jinaai/jina-embeddings-v2-base-code`).
    pub hf_repo: &'static str,
    /// Files to download: `(remote_path_under_repo_resolve_main, local_relative_path)`.
    pub artifacts: &'static [Artifact],
}

/// Default model: Jina v2 Base Code, 768-dimensional.
///
/// NOTE: artifact paths were confirmed against the HuggingFace repository
/// `jinaai/jina-embeddings-v2-base-code` as of 2026-05-31. The ONNX export
/// lives at `onnx/model.onnx` and the fast tokenizer at `tokenizer.json`.
/// Verify these paths manually with an `#[ignore]`d smoke test after a fresh
/// network run (`cargo test -- --ignored`).
pub static JINA_V2_BASE_CODE: ModelSpec = ModelSpec {
    model_name: "jinaai/jina-embeddings-v2-base-code",
    dim: 768,
    hf_repo: "jinaai/jina-embeddings-v2-base-code",
    artifacts: &[
        Artifact {
            remote_path: "tokenizer.json",
            local_path: "tokenizer.json",
        },
        Artifact {
            remote_path: "onnx/model.onnx",
            local_path: "onnx/model.onnx",
        },
    ],
};

/// Lite model: BAAI BGE-Small-EN-v1.5, 384-dimensional.
///
/// NOTE: artifact paths confirmed against `BAAI/bge-small-en-v1.5` on HuggingFace
/// as of 2026-05-31. Fast tokenizer at `tokenizer.json`, ONNX at `onnx/model.onnx`.
pub static BGE_SMALL_EN_V1_5: ModelSpec = ModelSpec {
    model_name: "BAAI/bge-small-en-v1.5",
    dim: 384,
    hf_repo: "BAAI/bge-small-en-v1.5",
    artifacts: &[
        Artifact {
            remote_path: "tokenizer.json",
            local_path: "tokenizer.json",
        },
        Artifact {
            remote_path: "onnx/model.onnx",
            local_path: "onnx/model.onnx",
        },
    ],
};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Download all artifacts for the given model spec into the local model cache.
///
/// Uses `base_url` as the HuggingFace base (default: `https://huggingface.co`).
/// Tests pass a local `wiremock` URL here so no real network traffic occurs.
///
/// Prints progress to **stderr** via `indicatif`.
pub async fn download_model(spec: &ModelSpec, config: &Config, base_url: &str) -> Result<()> {
    let model_dir = resolve_model_dir(spec.model_name, config)?;
    let client = Client::new();

    eprintln!(
        "Downloading model '{}' ({}d) into {}",
        spec.model_name,
        spec.dim,
        model_dir.display()
    );

    for artifact in spec.artifacts {
        let local_path = model_dir.join(artifact.local_path);
        // Ensure parent directory exists.
        if let Some(parent) = local_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let url = format!(
            "{}/resolve/main/{}",
            hf_repo_base(base_url, spec.hf_repo),
            artifact.remote_path
        );
        download_artifact(&client, &url, &local_path, artifact.local_path).await?;
    }

    eprintln!("Model '{}' is ready.", spec.model_name);
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build the `<base_url>/<hf_repo>` prefix for resolve URLs.
fn hf_repo_base(base_url: &str, hf_repo: &str) -> String {
    format!("{}/{}", base_url.trim_end_matches('/'), hf_repo)
}

/// Download a single artifact, with resumption and idempotency.
///
/// - If the final file exists and is non-empty: skip (idempotent).
/// - If a `.part` file exists: try range-resume.
/// - Otherwise: fresh download.
///
/// Progress is shown on stderr via `indicatif`.
async fn download_artifact(
    client: &Client,
    url: &str,
    dest: &Path,
    display_name: &str,
) -> Result<()> {
    // Idempotency: if the final file is already there and non-empty, skip.
    if dest.exists() {
        let existing_len = dest.metadata()?.len();
        if existing_len > 0 {
            eprintln!("  [skip] {display_name} already exists ({existing_len} bytes)");
            return Ok(());
        }
        // Zero-byte final file is an anomaly — delete and re-download.
        fs::remove_file(dest)?;
    }

    let part_path = part_file_path(dest);
    let existing_part_len = if part_path.exists() {
        let len = part_path.metadata()?.len();
        // Sanity-check: a part file larger than 4 GiB is suspicious — discard.
        if len > 4 * 1024 * 1024 * 1024 {
            tracing::warn!(
                part = %part_path.display(),
                len,
                "part file suspiciously large, discarding"
            );
            fs::remove_file(&part_path)?;
            0
        } else {
            len
        }
    } else {
        0
    };

    // --- issue the HTTP request ---
    let mut req = client.get(url);
    if existing_part_len > 0 {
        req = req.header(header::RANGE, format!("bytes={existing_part_len}-"));
    }

    let response = req.send().await.map_err(VektorError::Network)?;

    let status = response.status();

    // 404 / 5xx → clear error naming the path, no secret leakage.
    if !status.is_success() && status != StatusCode::PARTIAL_CONTENT {
        return Err(VektorError::Config(format!(
            "download failed for '{display_name}': HTTP {status} (url={url})"
        )));
    }

    // If we sent a range request but got 200 (no range support), discard the partial.
    let append = if existing_part_len > 0 && status == StatusCode::PARTIAL_CONTENT {
        true
    } else {
        if existing_part_len > 0 {
            // Server returned 200 instead of 206 — start fresh.
            tracing::debug!(
                url = %url,
                "server does not support range requests; restarting download"
            );
            fs::remove_file(&part_path)?;
        }
        false
    };

    // Total size for the progress bar (Content-Length is the remaining bytes
    // when a range was honoured; add existing_part_len to get the real total).
    let content_length = response.content_length().unwrap_or(0);
    let total = if append {
        existing_part_len + content_length
    } else {
        content_length
    };

    let pb = build_progress_bar(total, display_name);

    // Open or create/append the .part file.
    let mut file: File = if append {
        let f = OpenOptions::new().append(true).open(&part_path)?;
        pb.set_position(existing_part_len);
        f
    } else {
        File::create(&part_path)?
    };

    let mut response = response;
    while let Some(chunk) = response.chunk().await.map_err(VektorError::Network)? {
        file.write_all(&chunk)?;
        pb.inc(chunk.len() as u64);
    }
    file.flush()?;
    drop(file); // close before rename

    pb.finish_and_clear();

    // Atomic rename: .part → final path.
    fs::rename(&part_path, dest)?;
    tracing::debug!(dest = %dest.display(), "artifact installed");
    Ok(())
}

/// Path of the temporary in-progress file for `dest`.
fn part_file_path(dest: &Path) -> PathBuf {
    let mut name = dest
        .file_name()
        .expect("artifact path has a file name")
        .to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

/// Resolve `<data_dir>/models/<safe-model-name>/` using the same expansion
/// rules as [`crate::embedder::onnx`] (task 3.2) so paths match exactly.
fn resolve_model_dir(model_name: &str, config: &Config) -> Result<PathBuf> {
    let base = expand_data_dir(&config.index.data_dir)?;
    let safe = model_name.replace('/', "--");
    Ok(base.join("models").join(safe))
}

/// Construct an `indicatif` progress bar attached to **stderr**.
///
/// If `total == 0` the bar spins (unknown size); otherwise it shows bytes
/// transferred / total in human-readable form.
fn build_progress_bar(total: u64, display_name: &str) -> ProgressBar {
    let pb = if total > 0 {
        ProgressBar::new(total)
    } else {
        ProgressBar::new_spinner()
    };

    // indicatif writes to stderr by default when constructed via ProgressBar::new.
    // We force stderr explicitly to be safe.
    pb.set_draw_target(indicatif::ProgressDrawTarget::stderr());

    let style = ProgressStyle::with_template(
        "{msg} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})",
    )
    .unwrap_or_else(|_| ProgressStyle::default_bar())
    .progress_chars("=>-");

    pb.set_style(style);
    pb.set_message(format!("  {display_name}"));
    pb
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EmbeddingConfig, IndexConfig};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn config_for(data_dir: &Path) -> Config {
        Config {
            index: IndexConfig {
                data_dir: data_dir.to_string_lossy().into_owned(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn config_for_lite(data_dir: &Path) -> Config {
        Config {
            embedding: EmbeddingConfig {
                onnx_model: "BAAI/bge-small-en-v1.5".into(),
                ..Default::default()
            },
            index: IndexConfig {
                data_dir: data_dir.to_string_lossy().into_owned(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    // -----------------------------------------------------------------------
    // Unit tests — path helpers (no network)
    // -----------------------------------------------------------------------

    #[test]
    fn models_download_resolve_model_dir_jina_default() {
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let config = config_for(tmpdir.path());

        let dir =
            resolve_model_dir("jinaai/jina-embeddings-v2-base-code", &config).expect("resolve dir");

        assert_eq!(
            dir,
            tmpdir
                .path()
                .join("models")
                .join("jinaai--jina-embeddings-v2-base-code")
        );
    }

    #[test]
    fn models_download_resolve_model_dir_lite() {
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let config = config_for_lite(tmpdir.path());

        let dir = resolve_model_dir("BAAI/bge-small-en-v1.5", &config).expect("resolve dir");

        assert_eq!(
            dir,
            tmpdir.path().join("models").join("BAAI--bge-small-en-v1.5")
        );
    }

    #[test]
    fn models_download_part_file_path_appends_part_suffix() {
        let dest = PathBuf::from("/tmp/models/jinaai--jina/tokenizer.json");
        let part = part_file_path(&dest);
        assert_eq!(
            part,
            PathBuf::from("/tmp/models/jinaai--jina/tokenizer.json.part")
        );
    }

    #[test]
    fn models_download_hf_repo_base_builds_correct_url() {
        let base = hf_repo_base(
            "https://huggingface.co",
            "jinaai/jina-embeddings-v2-base-code",
        );
        assert_eq!(
            base,
            "https://huggingface.co/jinaai/jina-embeddings-v2-base-code"
        );
    }

    #[test]
    fn models_download_hf_repo_base_strips_trailing_slash() {
        let base = hf_repo_base("https://huggingface.co/", "BAAI/bge-small-en-v1.5");
        assert_eq!(base, "https://huggingface.co/BAAI/bge-small-en-v1.5");
    }

    #[test]
    fn models_download_static_table_jina_has_two_artifacts() {
        assert_eq!(JINA_V2_BASE_CODE.artifacts.len(), 2);
        assert_eq!(JINA_V2_BASE_CODE.dim, 768);
        assert!(
            JINA_V2_BASE_CODE
                .artifacts
                .iter()
                .any(|a| a.local_path == "tokenizer.json")
        );
        assert!(
            JINA_V2_BASE_CODE
                .artifacts
                .iter()
                .any(|a| a.local_path == "onnx/model.onnx")
        );
    }

    #[test]
    fn models_download_static_table_bge_has_two_artifacts() {
        assert_eq!(BGE_SMALL_EN_V1_5.artifacts.len(), 2);
        assert_eq!(BGE_SMALL_EN_V1_5.dim, 384);
        assert!(
            BGE_SMALL_EN_V1_5
                .artifacts
                .iter()
                .any(|a| a.local_path == "tokenizer.json")
        );
        assert!(
            BGE_SMALL_EN_V1_5
                .artifacts
                .iter()
                .any(|a| a.local_path == "onnx/model.onnx")
        );
    }

    // -----------------------------------------------------------------------
    // Integration tests — wiremock (local fixture server, no HuggingFace)
    // -----------------------------------------------------------------------

    /// Mount a wiremock route that serves `body` for `GET /<path>`.
    async fn mount_200(server: &MockServer, route_path: &str, body: Vec<u8>) {
        Mock::given(method("GET"))
            .and(path(route_path))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(body)
                    .insert_header("content-type", "application/octet-stream"),
            )
            .mount(server)
            .await;
    }

    /// Fresh download installs both artifacts in the correct directories.
    #[tokio::test]
    async fn models_download_fresh_download_installs_both_artifacts() {
        let server = MockServer::start().await;
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let config = config_for(tmpdir.path());

        // Serve tokenizer.json and onnx/model.onnx for the default (Jina) model.
        mount_200(
            &server,
            "/jinaai/jina-embeddings-v2-base-code/resolve/main/tokenizer.json",
            b"fake-tokenizer-content".to_vec(),
        )
        .await;
        mount_200(
            &server,
            "/jinaai/jina-embeddings-v2-base-code/resolve/main/onnx/model.onnx",
            b"fake-onnx-content".to_vec(),
        )
        .await;

        download_model(&JINA_V2_BASE_CODE, &config, &server.uri())
            .await
            .expect("download should succeed");

        let model_dir = tmpdir
            .path()
            .join("models")
            .join("jinaai--jina-embeddings-v2-base-code");

        assert!(
            model_dir.join("tokenizer.json").exists(),
            "tokenizer.json must be installed"
        );
        assert!(
            model_dir.join("onnx").join("model.onnx").exists(),
            "onnx/model.onnx must be installed"
        );

        // Verify content (not truncated or corrupted).
        let tok = fs::read(model_dir.join("tokenizer.json")).expect("read tokenizer");
        assert_eq!(tok, b"fake-tokenizer-content");
        let onnx = fs::read(model_dir.join("onnx").join("model.onnx")).expect("read onnx");
        assert_eq!(onnx, b"fake-onnx-content");
    }

    /// Lite download uses the BGE model dir.
    #[tokio::test]
    async fn models_download_lite_installs_under_bge_dir() {
        let server = MockServer::start().await;
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let config = config_for_lite(tmpdir.path());

        mount_200(
            &server,
            "/BAAI/bge-small-en-v1.5/resolve/main/tokenizer.json",
            b"bge-tokenizer".to_vec(),
        )
        .await;
        mount_200(
            &server,
            "/BAAI/bge-small-en-v1.5/resolve/main/onnx/model.onnx",
            b"bge-onnx".to_vec(),
        )
        .await;

        download_model(&BGE_SMALL_EN_V1_5, &config, &server.uri())
            .await
            .expect("lite download should succeed");

        let model_dir = tmpdir.path().join("models").join("BAAI--bge-small-en-v1.5");

        assert!(model_dir.join("tokenizer.json").exists());
        assert!(model_dir.join("onnx").join("model.onnx").exists());
    }

    /// Idempotent: if the final file exists and is non-empty, a second call must
    /// NOT issue a new GET (the file is unchanged and no second request is made).
    #[tokio::test]
    async fn models_download_idempotent_skips_existing_files() {
        let server = MockServer::start().await;
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let config = config_for(tmpdir.path());

        // Pre-populate both artifact files so the downloader should skip.
        let model_dir = tmpdir
            .path()
            .join("models")
            .join("jinaai--jina-embeddings-v2-base-code");
        fs::create_dir_all(model_dir.join("onnx")).expect("mkdir");
        fs::write(model_dir.join("tokenizer.json"), b"existing-tok").expect("write tok");
        fs::write(model_dir.join("onnx").join("model.onnx"), b"existing-onnx").expect("write onnx");

        // No mock routes registered — any GET would fail the test (404 wiremock error).
        // The downloader must return Ok without issuing any request.
        download_model(&JINA_V2_BASE_CODE, &config, &server.uri())
            .await
            .expect("idempotent call must succeed");

        // Files must remain unchanged.
        let tok = fs::read(model_dir.join("tokenizer.json")).expect("read");
        assert_eq!(tok, b"existing-tok");
    }

    /// Partial `.part` file + server returns HTTP 206 → append and succeed.
    #[tokio::test]
    async fn models_download_resume_206_appends_and_renames() {
        use wiremock::matchers::header_exists;

        let server = MockServer::start().await;
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let config = config_for(tmpdir.path());

        let model_dir = tmpdir
            .path()
            .join("models")
            .join("jinaai--jina-embeddings-v2-base-code");
        fs::create_dir_all(&model_dir).expect("mkdir");

        // Pre-create a .part file with the first half of the content.
        let first_half = b"AAAA";
        let second_half = b"BBBB";
        let part_path = model_dir.join("tokenizer.json.part");
        fs::write(&part_path, first_half).expect("write part");

        // Mount a 206 response for the Range request for tokenizer.json.
        Mock::given(method("GET"))
            .and(path(
                "/jinaai/jina-embeddings-v2-base-code/resolve/main/tokenizer.json",
            ))
            .and(header_exists("range"))
            .respond_with(
                ResponseTemplate::new(206)
                    .set_body_bytes(second_half.to_vec())
                    .insert_header("content-type", "application/octet-stream"),
            )
            .mount(&server)
            .await;

        // onnx/model.onnx also needs a fresh (no range) download.
        mount_200(
            &server,
            "/jinaai/jina-embeddings-v2-base-code/resolve/main/onnx/model.onnx",
            b"fake-onnx".to_vec(),
        )
        .await;

        download_model(&JINA_V2_BASE_CODE, &config, &server.uri())
            .await
            .expect("resumed download should succeed");

        // tokenizer.json must be the concatenation of first_half + second_half.
        let tok = fs::read(model_dir.join("tokenizer.json")).expect("read");
        let mut expected = Vec::new();
        expected.extend_from_slice(first_half);
        expected.extend_from_slice(second_half);
        assert_eq!(
            tok, expected,
            "resumed content must be first_half + second_half"
        );

        // No .part file should remain.
        assert!(
            !part_path.exists(),
            "part file must be cleaned up after successful rename"
        );
    }

    /// If the server returns 200 in response to a range request (no range
    /// support), the partial is discarded and the full download proceeds cleanly.
    #[tokio::test]
    async fn models_download_server_ignores_range_restarts_cleanly() {
        use wiremock::matchers::header_exists;

        let server = MockServer::start().await;
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let config = config_for(tmpdir.path());

        let model_dir = tmpdir
            .path()
            .join("models")
            .join("jinaai--jina-embeddings-v2-base-code");
        fs::create_dir_all(&model_dir).expect("mkdir");

        // Pre-create a stale .part file.
        let part_path = model_dir.join("tokenizer.json.part");
        fs::write(&part_path, b"stale-partial").expect("write stale part");

        let full_body = b"FULL-TOKENIZER-CONTENT";

        // Server returns 200 (not 206) even when a Range header is sent.
        Mock::given(method("GET"))
            .and(path(
                "/jinaai/jina-embeddings-v2-base-code/resolve/main/tokenizer.json",
            ))
            .and(header_exists("range"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(full_body.to_vec())
                    .insert_header("content-type", "application/octet-stream"),
            )
            .mount(&server)
            .await;

        mount_200(
            &server,
            "/jinaai/jina-embeddings-v2-base-code/resolve/main/onnx/model.onnx",
            b"onnx-content".to_vec(),
        )
        .await;

        download_model(&JINA_V2_BASE_CODE, &config, &server.uri())
            .await
            .expect("restart download should succeed");

        let tok = fs::read(model_dir.join("tokenizer.json")).expect("read");
        assert_eq!(tok, full_body, "full body must be written after restart");

        // .part must be gone.
        assert!(
            !part_path.exists(),
            "stale part file must be removed after successful download"
        );
    }

    /// HTTP 404 from the server produces a clear error that names the artifact path.
    #[tokio::test]
    async fn models_download_http_404_yields_clear_error() {
        let server = MockServer::start().await;
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let config = config_for(tmpdir.path());

        // tokenizer.json returns 404.
        Mock::given(method("GET"))
            .and(path(
                "/jinaai/jina-embeddings-v2-base-code/resolve/main/tokenizer.json",
            ))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let err = download_model(&JINA_V2_BASE_CODE, &config, &server.uri())
            .await
            .expect_err("404 must produce an error");

        let msg = err.to_string();
        assert!(
            msg.contains("tokenizer.json"),
            "error must name the artifact: {msg}"
        );
        assert!(
            msg.contains("404"),
            "error must include the HTTP status code: {msg}"
        );
        // Must NOT contain any secret or credential.
        assert!(!msg.contains("sk-"), "no API key in error: {msg}");
    }

    /// HTTP 500 from the server also produces a clear, non-panicking error.
    #[tokio::test]
    async fn models_download_http_500_yields_clear_error() {
        let server = MockServer::start().await;
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let config = config_for(tmpdir.path());

        Mock::given(method("GET"))
            .and(path(
                "/jinaai/jina-embeddings-v2-base-code/resolve/main/tokenizer.json",
            ))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let err = download_model(&JINA_V2_BASE_CODE, &config, &server.uri())
            .await
            .expect_err("500 must produce an error");

        let msg = err.to_string();
        assert!(
            msg.contains("500") || msg.contains("tokenizer.json"),
            "error: {msg}"
        );
    }

    /// Zero-byte final file is treated as incomplete and re-downloaded.
    #[tokio::test]
    async fn models_download_zero_byte_final_file_is_redownloaded() {
        let server = MockServer::start().await;
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let config = config_for(tmpdir.path());

        let model_dir = tmpdir
            .path()
            .join("models")
            .join("jinaai--jina-embeddings-v2-base-code");
        fs::create_dir_all(model_dir.join("onnx")).expect("mkdir");

        // Write zero-byte tokenizer.json (truncation artifact).
        fs::write(model_dir.join("tokenizer.json"), b"").expect("write empty tok");
        fs::write(model_dir.join("onnx").join("model.onnx"), b"existing-onnx").expect("write onnx");

        mount_200(
            &server,
            "/jinaai/jina-embeddings-v2-base-code/resolve/main/tokenizer.json",
            b"fresh-tokenizer-content".to_vec(),
        )
        .await;

        // onnx/model.onnx is non-zero, so it must NOT issue a request.
        // (No mock route for it → any GET would produce a 404 and fail the test.)

        download_model(&JINA_V2_BASE_CODE, &config, &server.uri())
            .await
            .expect("re-download of zero-byte file should succeed");

        let tok = fs::read(model_dir.join("tokenizer.json")).expect("read");
        assert_eq!(tok, b"fresh-tokenizer-content");
        // onnx must remain unchanged.
        let onnx = fs::read(model_dir.join("onnx").join("model.onnx")).expect("read onnx");
        assert_eq!(onnx, b"existing-onnx");
    }

    // -----------------------------------------------------------------------
    // Ignored smoke tests — require real HuggingFace network access
    // -----------------------------------------------------------------------

    /// Smoke test: downloads real artifacts from HuggingFace into a temp dir and
    /// verifies both files are present and non-empty.
    ///
    /// NOTE: artifact paths (`tokenizer.json`, `onnx/model.onnx`) must exist on
    /// the `jinaai/jina-embeddings-v2-base-code` HF repo. Verify manually if
    /// HuggingFace reorganizes the repo.
    ///
    /// Run: `cargo test models_download -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "requires HuggingFace network access; run manually to verify real artifact paths"]
    async fn models_download_smoke_real_hf_jina_default() {
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let config = config_for(tmpdir.path());

        download_model(&JINA_V2_BASE_CODE, &config, "https://huggingface.co")
            .await
            .expect("real HuggingFace download should succeed");

        let model_dir = tmpdir
            .path()
            .join("models")
            .join("jinaai--jina-embeddings-v2-base-code");
        assert!(model_dir.join("tokenizer.json").exists());
        assert!(model_dir.join("onnx").join("model.onnx").exists());
        assert!(
            model_dir.join("tokenizer.json").metadata().unwrap().len() > 0,
            "tokenizer.json must not be empty"
        );
        assert!(
            model_dir
                .join("onnx")
                .join("model.onnx")
                .metadata()
                .unwrap()
                .len()
                > 0,
            "onnx/model.onnx must not be empty"
        );
    }

    /// Smoke test: downloads real BGE-small artifacts from HuggingFace.
    ///
    /// Run: `cargo test models_download -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "requires HuggingFace network access; run manually to verify real artifact paths"]
    async fn models_download_smoke_real_hf_bge_lite() {
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let config = config_for_lite(tmpdir.path());

        download_model(&BGE_SMALL_EN_V1_5, &config, "https://huggingface.co")
            .await
            .expect("real HuggingFace lite download should succeed");

        let model_dir = tmpdir.path().join("models").join("BAAI--bge-small-en-v1.5");
        assert!(model_dir.join("tokenizer.json").exists());
        assert!(model_dir.join("onnx").join("model.onnx").exists());
    }
}
