use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Result, VektorError};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub embedding: EmbeddingConfig,
    pub index: IndexConfig,
    pub watcher: WatcherConfig,
    pub server: ServerConfig,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbeddingConfig {
    pub backend: String,
    pub openai_api_key: String,
    pub openai_base_url: String,
    pub openai_model: String,
    pub ollama_url: String,
    pub ollama_model: String,
    pub onnx_model: String,
    pub fallback_to_onnx: bool,
    pub max_requests_per_minute: u32,
}

impl std::fmt::Debug for EmbeddingConfig {
    /// Manual `Debug` redacts `openai_api_key` so the secret never leaks via
    /// `tracing::debug!(?config, ...)` (the config-loaded log in `cli::run`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingConfig")
            .field("backend", &self.backend)
            .field("openai_api_key", &"<redacted>")
            .field("openai_base_url", &self.openai_base_url)
            .field("openai_model", &self.openai_model)
            .field("ollama_url", &self.ollama_url)
            .field("ollama_model", &self.ollama_model)
            .field("onnx_model", &self.onnx_model)
            .field("fallback_to_onnx", &self.fallback_to_onnx)
            .field("max_requests_per_minute", &self.max_requests_per_minute)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexConfig {
    pub data_dir: String,
    pub max_file_size_kb: u64,
    pub chunk_max_lines: usize,
    pub chunk_overlap_pct: u8,
    pub doc_chunk_max_lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WatcherConfig {
    pub debounce_ms: u64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub mode: String,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            backend: "onnx".into(),
            openai_api_key: String::new(),
            openai_base_url: "https://api.openai.com/v1".into(),
            openai_model: "text-embedding-3-small".into(),
            ollama_url: "http://localhost:11434".into(),
            ollama_model: "nomic-embed-text".into(),
            onnx_model: "jinaai/jina-embeddings-v2-base-code".into(),
            fallback_to_onnx: true,
            max_requests_per_minute: 500,
        }
    }
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            data_dir: "~/.vektor".into(),
            max_file_size_kb: 512,
            chunk_max_lines: 200,
            chunk_overlap_pct: 25,
            doc_chunk_max_lines: 40,
        }
    }
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 200,
            enabled: true,
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            mode: "stdio".into(),
        }
    }
}

impl Config {
    pub fn load(override_path: Option<PathBuf>) -> Result<Self> {
        let defaults =
            config::Config::try_from(&Self::default()).map_err(config_error_to_vektor)?;
        let mut builder = config::Config::builder().add_source(defaults);

        let explicit_override = override_path.is_some();
        let file_path = override_path
            .or_else(|| dirs::home_dir().map(|home| home.join(".vektor").join("config.toml")));

        if let Some(path) = file_path {
            if path.exists() {
                builder = builder.add_source(config::File::from(path));
            } else if explicit_override {
                return Err(VektorError::Config(format!(
                    "config file not found: {}",
                    path.display()
                )));
            }
        }

        builder = builder.add_source(
            config::Environment::with_prefix("VEKTOR")
                .prefix_separator("__")
                .separator("__")
                .convert_case(config::Case::Snake),
        );

        builder
            .build()
            .and_then(|config| config.try_deserialize::<Self>())
            .map_err(config_error_to_vektor)
    }
}

fn config_error_to_vektor(error: config::ConfigError) -> VektorError {
    VektorError::Config(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_config_debug_redacts_api_key() {
        let cfg = EmbeddingConfig {
            openai_api_key: "sk-super-secret-12345".into(),
            ..Default::default()
        };
        let rendered = format!("{cfg:?}");
        assert!(
            !rendered.contains("sk-super-secret-12345"),
            "openai_api_key leaked in Debug output: {rendered}"
        );
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();

        assert_eq!(config.embedding.backend, "onnx");
        assert_eq!(config.embedding.openai_api_key, "");
        assert_eq!(
            config.embedding.openai_base_url,
            "https://api.openai.com/v1"
        );
        assert_eq!(config.embedding.openai_model, "text-embedding-3-small");
        assert_eq!(config.embedding.ollama_url, "http://localhost:11434");
        assert_eq!(config.embedding.ollama_model, "nomic-embed-text");
        assert_eq!(
            config.embedding.onnx_model,
            "jinaai/jina-embeddings-v2-base-code"
        );
        assert!(config.embedding.fallback_to_onnx);
        assert_eq!(config.embedding.max_requests_per_minute, 500);
        assert_eq!(config.index.data_dir, "~/.vektor");
        assert_eq!(config.index.max_file_size_kb, 512);
        assert_eq!(config.index.chunk_max_lines, 200);
        assert_eq!(config.index.chunk_overlap_pct, 25);
        assert_eq!(config.index.doc_chunk_max_lines, 40);
        assert_eq!(config.watcher.debounce_ms, 200);
        assert!(config.watcher.enabled);
        assert_eq!(config.server.mode, "stdio");
    }

    #[test]
    fn test_load_from_file() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let path = tempdir.path().join("config.toml");
        std::fs::write(&path, "[embedding]\nbackend = \"openai\"\n").expect("write config file");

        let config = temp_env::with_vars(cleared_vektor_env(), || {
            Config::load(Some(path)).expect("load config")
        });

        assert_eq!(config.embedding.backend, "openai");
        assert_eq!(config.index.chunk_max_lines, 200);
    }

    #[test]
    fn test_load_from_file_observes_config_override() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let path = tempdir.path().join("config.toml");
        std::fs::write(&path, "[embedding]\nbackend = \"ollama\"\n").expect("write config file");

        let config = temp_env::with_vars(cleared_vektor_env(), || {
            Config::load(Some(path)).expect("load config")
        });

        assert_eq!(config.embedding.backend, "ollama");
    }

    #[test]
    fn test_explicit_override_path_missing() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let missing = tempdir.path().join("missing.toml");

        let error = temp_env::with_vars(cleared_vektor_env(), || {
            Config::load(Some(missing)).expect_err("missing explicit config should error")
        });

        assert!(matches!(error, VektorError::Config(_)));
        assert!(error.to_string().contains("config file not found"));
    }

    #[test]
    fn test_malformed_config_file_errors() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let path = tempdir.path().join("config.toml");
        std::fs::write(&path, "[embedding\nbackend = \"openai\"\n")
            .expect("write malformed config file");

        let error = temp_env::with_vars(cleared_vektor_env(), || {
            Config::load(Some(path)).expect_err("malformed config should error")
        });

        assert!(matches!(error, VektorError::Config(_)));
    }

    #[test]
    fn test_missing_default_config_uses_defaults() {
        let fake_home = tempfile::tempdir().expect("create fake home");

        temp_env::with_vars(isolated_env(fake_home.path(), &[]), || {
            let config = Config::load(None).expect("load defaults");

            assert_eq!(config.embedding.backend, "onnx");
            assert_eq!(config.server.mode, "stdio");
        });
    }

    #[test]
    fn test_env_override() {
        let fake_home = tempfile::tempdir().expect("create fake home");

        temp_env::with_vars(
            isolated_env(
                fake_home.path(),
                &[("VEKTOR__EMBEDDING__BACKEND", "ollama")],
            ),
            || {
                let config = Config::load(None).expect("load config");

                assert_eq!(config.embedding.backend, "ollama");
            },
        );
    }

    #[test]
    fn test_env_compound_key_override() {
        let fake_home = tempfile::tempdir().expect("create fake home");

        temp_env::with_vars(
            isolated_env(
                fake_home.path(),
                &[("VEKTOR__EMBEDDING__OPENAI_API_KEY", "sk-test")],
            ),
            || {
                let config = Config::load(None).expect("load config");

                assert_eq!(config.embedding.openai_api_key, "sk-test");
            },
        );
    }

    fn isolated_env(
        fake_home: &std::path::Path,
        overrides: &[(&'static str, &'static str)],
    ) -> Vec<(&'static str, Option<String>)> {
        let mut vars = cleared_vektor_env();
        let home = fake_home.to_string_lossy().into_owned();

        vars.push(("HOME", Some(home.clone())));
        vars.push(("USERPROFILE", Some(home)));

        for (key, value) in overrides {
            vars.push((*key, Some((*value).to_string())));
        }

        vars
    }

    fn cleared_vektor_env() -> Vec<(&'static str, Option<String>)> {
        VEKTOR_ENV_KEYS.iter().map(|key| (*key, None)).collect()
    }

    const VEKTOR_ENV_KEYS: &[&str] = &[
        "VEKTOR__EMBEDDING__BACKEND",
        "VEKTOR__EMBEDDING__OPENAI_API_KEY",
        "VEKTOR__EMBEDDING__OPENAI_BASE_URL",
        "VEKTOR__EMBEDDING__OPENAI_MODEL",
        "VEKTOR__EMBEDDING__OLLAMA_URL",
        "VEKTOR__EMBEDDING__OLLAMA_MODEL",
        "VEKTOR__EMBEDDING__ONNX_MODEL",
        "VEKTOR__EMBEDDING__FALLBACK_TO_ONNX",
        "VEKTOR__EMBEDDING__MAX_REQUESTS_PER_MINUTE",
        "VEKTOR__INDEX__DATA_DIR",
        "VEKTOR__INDEX__MAX_FILE_SIZE_KB",
        "VEKTOR__INDEX__CHUNK_MAX_LINES",
        "VEKTOR__INDEX__CHUNK_OVERLAP_PCT",
        "VEKTOR__INDEX__DOC_CHUNK_MAX_LINES",
        "VEKTOR__WATCHER__DEBOUNCE_MS",
        "VEKTOR__WATCHER__ENABLED",
        "VEKTOR__SERVER__MODE",
    ];
}
