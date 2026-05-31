//! Secret detection: file-level skip list and content-level regex + entropy checks.
//!
//! # Design
//! - [`SecretDetector::should_skip_file`] is path-only, performs zero I/O, and runs
//!   **before** bytes are read so secret files never enter memory.
//! - [`SecretDetector::contains_secret`] scans chunk content after safe files are
//!   read and chunked, structured so task 3.7c can call it per-chunk right before
//!   embedding. Neither function logs secret values; callers log paths/counts only.
//! - The rule set is static and local (no external scanners).

use regex::Regex;
use std::sync::OnceLock;

/// Compiled regex patterns for content-level secret detection.
struct Patterns {
    pem_header: Regex,
    aws_access_key: Regex,
    github_token: Regex,
    slack_token: Regex,
    /// Quoted form: `SECRET_KEY = "value"` / `secret: 'value'`
    high_entropy_assignment_quoted: Regex,
    /// Unquoted form: `AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI...` (dotenv/shell style)
    high_entropy_assignment_unquoted: Regex,
    jwt: Regex,
    google_api_key: Regex,
    stripe_key: Regex,
    openai_key: Regex,
}

impl Patterns {
    fn new() -> Self {
        Self {
            // PEM private key header — matches ALL private-key header types.
            // Examples: RSA PRIVATE KEY, EC PRIVATE KEY, ENCRYPTED PRIVATE KEY,
            //           PGP PRIVATE KEY BLOCK, OPENSSH PRIVATE KEY, PRIVATE KEY.
            pem_header: Regex::new(
                r"-----BEGIN (?:[A-Z0-9 ]*PRIVATE KEY(?:[A-Z0-9 ]*)?)-----",
            )
            .expect("valid pem regex"),
            // AWS access key ID — 20 uppercase alphanumeric chars starting with AKIA/ASIA/AROA/…
            aws_access_key: Regex::new(
                r"\b(AKIA|ASIA|AROA|AIDA|AIPA|ANPA|ANVA|APKA)[A-Z0-9]{16}\b",
            )
            .expect("valid aws regex"),
            // GitHub personal access tokens (classic ghp_, gho_, ghu_, ghs_, ghr_, and fine-grained github_pat_)
            github_token: Regex::new(
                r"\b(ghp_[A-Za-z0-9_]{36,}|gho_[A-Za-z0-9_]{36,}|ghu_[A-Za-z0-9_]{36,}|ghs_[A-Za-z0-9_]{36,}|ghr_[A-Za-z0-9_]{36,}|github_pat_[A-Za-z0-9_]{82,})\b",
            )
            .expect("valid github regex"),
            // Slack bot/user/app/workspace tokens
            slack_token: Regex::new(r"\b(xox[baprs]-[A-Za-z0-9\-]{10,})\b")
                .expect("valid slack regex"),

            // Generic high-entropy assignment — QUOTED form.
            //
            // Keyword matching rules (Fix M-1):
            // - Optional `<prefix>_` before the keyword
            // - Keyword must END the identifier (no suffix word-chars follow)
            // - Recognised keywords: token, secret, key, password, passwd, pwd,
            //   apikey, api_key, credential, auth
            //
            // Example matches:  SECRET_KEY = "…"   API_KEY: '…'   db_password = "…"
            // Example non-matches: KEYBOARD_LAYOUT = "…"  token_url = "…"  authority = "…"
            //
            // `m` flag: multiline — `$` anchors to end-of-line (symmetric with unquoted).
            high_entropy_assignment_quoted: Regex::new(
                r#"(?im)(?:[A-Za-z0-9]+[_-])?(?:token|secret|key|password|passwd|pwd|apikey|api_key|credential|auth)\s*[:=]\s*["']([A-Za-z0-9+/=\-_.~!@#$%^&*]{16,})["']"#,
            )
            .expect("valid quoted entropy assignment regex"),

            // Unquoted form — dotenv/shell style without quotes.
            // Matches:  AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI...
            //           API_KEY=longsecretvalue1234
            // Value: 20+ chars, no spaces, secret-ish charset.
            //
            // `m` flag: multiline — `$` anchors to end-of-line, not end-of-string.
            // Without this, mid-chunk secrets (not on the last line) are silently missed.
            high_entropy_assignment_unquoted: Regex::new(
                r#"(?im)(?:[A-Za-z0-9]+[_-])?(?:token|secret|key|password|passwd|pwd|apikey|api_key|credential|auth)\s*[:=]\s*([A-Za-z0-9+/=\-_.~!@#$%^&*]{20,})\s*$"#,
            )
            .expect("valid unquoted entropy assignment regex"),

            // JWT: three base64url segments separated by dots.
            jwt: Regex::new(r"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+")
                .expect("valid jwt regex"),

            // Google API key: AIza followed by 35 alphanumeric/underscore/dash chars.
            google_api_key: Regex::new(r"AIza[0-9A-Za-z_\-]{35}")
                .expect("valid google api key regex"),

            // Stripe secret/restricted/publishable key.
            stripe_key: Regex::new(r"\b(sk|rk|pk)_(live|test)_[A-Za-z0-9]{16,}\b")
                .expect("valid stripe regex"),

            // OpenAI API key.
            openai_key: Regex::new(r"\bsk-(proj-)?[A-Za-z0-9_\-]{20,}\b")
                .expect("valid openai regex"),
        }
    }
}

static PATTERNS: OnceLock<Patterns> = OnceLock::new();

fn patterns() -> &'static Patterns {
    PATTERNS.get_or_init(Patterns::new)
}

/// File-name patterns that identify files that must be skipped at the path level,
/// before their bytes are ever read.
///
/// The list is intentionally conservative: we only skip files whose primary purpose
/// is credential storage, not any file that could hypothetically contain a secret.
const SECRET_FILENAME_PATTERNS: &[&str] = &[
    // Dot-env family: exact ".env" — see also the `.env.*` logic in `should_skip_file`.
    ".env",
    // Private key files
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    "id_ecdsa_sk",
    "id_ed25519_sk",
    // npm/yarn/pnpm auth files (may contain registry auth tokens)
    ".npmrc",
    ".yarnrc",
    ".yarnrc.yml",
    ".pnpmfile.cjs",
];

/// Extension patterns that identify secret files by their file extension.
const SECRET_EXTENSIONS: &[&str] = &[
    "pem", // PEM-encoded key/cert
    "key", // Private key
    "p12", // PKCS#12 keystore
    "pfx", // PKCS#12 keystore (Windows name)
    "jks", // Java KeyStore
];

/// Relative path components that indicate a credential directory/file.
const SECRET_PATH_COMPONENTS: &[&str] = &[
    ".aws/credentials",
    ".aws/config", // may contain role credentials
    ".ssh/id_rsa",
    ".ssh/id_dsa",
    ".ssh/id_ecdsa",
    ".ssh/id_ed25519",
    ".ssh/id_ecdsa_sk",
    ".ssh/id_ed25519_sk",
    "client_secret.json",   // Google OAuth
    "service_account.json", // Google service account (common name)
    "gcloud/credentials.db",
    ".config/gcloud/credentials.db",
    ".kube/config", // Kubernetes config (may contain cluster tokens)
    "secrets.yml",
    "secrets.yaml",
];

/// `.env.*` suffixes that are intentionally-committed placeholder/example files and
/// must **not** be skipped at the file level.
///
/// Defence-in-depth: the content-level `contains_secret` still runs on these files,
/// so a real secret pasted into `.env.example` is still caught.
const SAFE_ENV_SUFFIXES: &[&str] = &[
    ".env.example",
    ".env.sample",
    ".env.template",
    ".env.dist",
    ".env.defaults",
];

/// Detects secrets at both the file level (path-based, no I/O) and the content
/// level (regex + entropy).
///
/// Designed for reuse by task 3.7c: construct once with [`SecretDetector::new`]
/// and call [`should_skip_file`](SecretDetector::should_skip_file) and
/// [`contains_secret`](SecretDetector::contains_secret) from the indexing pipeline.
pub struct SecretDetector;

impl SecretDetector {
    /// Create a new `SecretDetector`. Compiles regex patterns on first call (lazy
    /// singleton); subsequent `new()` calls are cheap.
    pub fn new() -> Self {
        // Trigger compilation of the singleton now so callers see any pattern
        // errors at construction time rather than mid-indexing.
        let _ = patterns();
        Self
    }

    /// Returns `true` if the file at the given **relative path** should be skipped
    /// based on its name/extension alone.
    ///
    /// This function performs **zero I/O**. It is designed to be called *before*
    /// `read_file_lossy` so secret files are never read into memory.
    ///
    /// `rel_path` uses forward-slash separators (as produced by the discovery layer).
    pub fn should_skip_file(&self, rel_path: &str) -> bool {
        // Normalise to lowercase path segments for matching.
        let path_lower = rel_path.to_lowercase();

        // 1. Check against full path component patterns (e.g. ".aws/credentials").
        for pattern in SECRET_PATH_COMPONENTS {
            if path_lower.ends_with(pattern) || path_lower.contains(&format!("/{pattern}")) {
                return true;
            }
        }

        // 2. Extract the file name (last path component).
        let filename = rel_path
            .rsplit('/')
            .next()
            .unwrap_or(rel_path)
            .to_lowercase();

        // 3. Exact filename matches.
        if SECRET_FILENAME_PATTERNS.contains(&filename.as_str()) {
            return true;
        }

        // 4. `.env.*` family: starts with ".env." (e.g. ".env.local", ".env.production").
        //    Exception: known-safe placeholder suffixes (.env.example, .env.sample, etc.)
        //    are intentionally-committed files and should be indexed. The content-level
        //    `contains_secret` check still applies to them as defence-in-depth.
        if filename.starts_with(".env.") {
            let is_safe_suffix = SAFE_ENV_SUFFIXES.iter().any(|safe| filename == *safe);
            if !is_safe_suffix {
                return true;
            }
        }

        // 5. Extension matches.
        if let Some(ext) = filename.rsplit('.').next()
            && filename.len() > ext.len()
            && SECRET_EXTENSIONS.contains(&ext)
        {
            return true;
        }

        false
    }

    /// Returns `true` if the given chunk **content** appears to contain a secret value.
    ///
    /// Checks (in order):
    /// 1. PEM private key header (all types).
    /// 2. AWS access key ID (`AKIA…`).
    /// 3. GitHub personal access token (`ghp_…`, `github_pat_…`).
    /// 4. Slack token (`xox[baprs]-…`).
    /// 5. JWT (`eyJ…`).
    /// 6. Google API key (`AIza…`).
    /// 7. Stripe key (`sk_live_…`, `sk_test_…`, …).
    /// 8. OpenAI key (`sk-…`, `sk-proj-…`).
    /// 9. High-entropy assignment — quoted or unquoted forms.
    ///
    /// When a pattern matches, the function returns `true` immediately without
    /// capturing or returning the secret value.
    pub fn contains_secret(&self, content: &str) -> bool {
        let p = patterns();

        if p.pem_header.is_match(content) {
            return true;
        }
        if p.aws_access_key.is_match(content) {
            return true;
        }
        if p.github_token.is_match(content) {
            return true;
        }
        if p.slack_token.is_match(content) {
            return true;
        }
        if p.jwt.is_match(content) {
            return true;
        }
        if p.google_api_key.is_match(content) {
            return true;
        }
        if p.stripe_key.is_match(content) {
            return true;
        }
        if p.openai_key.is_match(content) {
            return true;
        }

        // For high-entropy assignments: match the pattern and check Shannon entropy
        // of the captured value to reduce false positives on short/low-entropy values.
        // Additionally require character diversity (≥2 distinct character classes)
        // to reject all-lowercase-alpha runs like keyboard walks.
        for cap in p.high_entropy_assignment_quoted.captures_iter(content) {
            if let Some(value) = cap.get(1)
                && is_high_entropy_secret(value.as_str())
            {
                return true;
            }
        }
        for cap in p.high_entropy_assignment_unquoted.captures_iter(content) {
            if let Some(value) = cap.get(1)
                && is_high_entropy_secret(value.as_str())
            {
                return true;
            }
        }

        false
    }
}

impl Default for SecretDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Minimum Shannon entropy (bits per character) for a value to be considered
/// high-entropy. Raised from 3.5 to 4.0 to reduce false positives on
/// common English strings and keyboard-walk sequences.
///
/// Reference points:
/// - Random 32-char base64 ≈ 5.0–5.5 bits/char
/// - Real API keys (mixed case + digits + symbols) ≈ 4.5–5.5 bits/char
/// - English words / keyboard walks ≈ 3.0–4.3 bits/char
const ENTROPY_THRESHOLD: f64 = 4.0;

/// Returns `true` if the string has high Shannon entropy **and** sufficient character
/// diversity to be a plausible secret value.
///
/// Character diversity rule: the value must contain characters from at least **2**
/// of the four classes {lowercase, uppercase, digit, symbol}. This kills
/// all-lowercase keyboard walks (`qwertyuiopasdfghjkl` scores 4.248 but is a single
/// class) and plain-English sentences even when they pass the entropy gate.
fn is_high_entropy_secret(s: &str) -> bool {
    if shannon_entropy(s) < ENTROPY_THRESHOLD {
        return false;
    }
    // Count character classes present in the value.
    let has_lower = s.bytes().any(|b| b.is_ascii_lowercase());
    let has_upper = s.bytes().any(|b| b.is_ascii_uppercase());
    let has_digit = s.bytes().any(|b| b.is_ascii_digit());
    let has_symbol = s
        .bytes()
        .any(|b| b.is_ascii_punctuation() || b == b'+' || b == b'/' || b == b'=');

    let class_count = [has_lower, has_upper, has_digit, has_symbol]
        .iter()
        .filter(|&&v| v)
        .count();

    class_count >= 2
}

/// Compute Shannon entropy (bits per character) of a string.
fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let len = s.len() as f64;
    let mut counts = [0u32; 256];
    for b in s.bytes() {
        counts[b as usize] += 1;
    }
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / len;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detector() -> SecretDetector {
        SecretDetector::new()
    }

    // ── File-level skip tests ──────────────────────────────────────────────────

    #[test]
    fn skip_dot_env_exact() {
        let d = detector();
        // Must skip — no I/O occurs; this proves the check is path-only.
        assert!(d.should_skip_file(".env"));
        assert!(d.should_skip_file("subdir/.env"));
    }

    #[test]
    fn skip_dot_env_variants() {
        let d = detector();
        assert!(d.should_skip_file(".env.local"));
        assert!(d.should_skip_file(".env.production"));
        assert!(d.should_skip_file(".env.test"));
        assert!(d.should_skip_file("config/.env.staging"));
    }

    /// `.env.example`, `.env.sample`, `.env.template`, `.env.dist`, `.env.defaults`
    /// are intentionally-committed placeholder files and must NOT be file-skipped.
    /// Content-level detection still applies to them (defence-in-depth).
    #[test]
    fn safe_env_placeholders_not_file_skipped() {
        let d = detector();
        assert!(!d.should_skip_file(".env.example"));
        assert!(!d.should_skip_file(".env.sample"));
        assert!(!d.should_skip_file(".env.template"));
        assert!(!d.should_skip_file(".env.dist"));
        assert!(!d.should_skip_file(".env.defaults"));
        // Also check in a subdir.
        assert!(!d.should_skip_file("config/.env.example"));
    }

    #[test]
    fn skip_pem_and_key_files() {
        let d = detector();
        assert!(d.should_skip_file("server.pem"));
        assert!(d.should_skip_file("certs/server.pem"));
        assert!(d.should_skip_file("private.key"));
        assert!(d.should_skip_file("keystore.p12"));
        assert!(d.should_skip_file("keystore.pfx"));
        assert!(d.should_skip_file("store.jks"));
    }

    #[test]
    fn skip_private_key_filenames() {
        let d = detector();
        assert!(d.should_skip_file("id_rsa"));
        assert!(d.should_skip_file(".ssh/id_rsa"));
        assert!(d.should_skip_file("id_ed25519"));
        assert!(d.should_skip_file("id_ecdsa"));
    }

    #[test]
    fn skip_npm_auth_files() {
        let d = detector();
        assert!(d.should_skip_file(".npmrc"));
        assert!(d.should_skip_file("frontend/.npmrc"));
        assert!(d.should_skip_file(".yarnrc"));
        assert!(d.should_skip_file(".yarnrc.yml"));
    }

    #[test]
    fn skip_aws_credentials_path() {
        let d = detector();
        assert!(d.should_skip_file(".aws/credentials"));
        assert!(d.should_skip_file("home/.aws/credentials"));
    }

    #[test]
    fn skip_ssh_key_paths() {
        let d = detector();
        assert!(d.should_skip_file(".ssh/id_rsa"));
        assert!(d.should_skip_file(".ssh/id_ed25519"));
    }

    #[test]
    fn skip_google_oauth_json() {
        let d = detector();
        assert!(d.should_skip_file("client_secret.json"));
        assert!(d.should_skip_file("auth/client_secret.json"));
    }

    #[test]
    fn skip_kubernetes_config() {
        let d = detector();
        assert!(d.should_skip_file(".kube/config"));
    }

    /// Core proof that file-level skip is enforced BEFORE any I/O:
    /// `should_skip_file` only takes a &str — it cannot open files.
    /// If it returns true for a path, the indexing loop never calls `read_file_lossy`.
    /// This test asserts the return value for a path that would contain secrets,
    /// without creating or reading any file.
    #[test]
    fn file_level_skip_requires_no_io() {
        let d = detector();
        // These paths need not exist on disk — the function never touches the FS.
        assert!(d.should_skip_file(".env"));
        assert!(d.should_skip_file(".env.production"));
        assert!(d.should_skip_file("secrets.yml"));
        assert!(d.should_skip_file("id_rsa"));
        assert!(d.should_skip_file("private.key"));
        assert!(d.should_skip_file(".npmrc"));
        // A normal source file must NOT be skipped.
        assert!(!d.should_skip_file("src/main.rs"));
    }

    #[test]
    fn safe_files_are_not_skipped() {
        let d = detector();
        assert!(!d.should_skip_file("src/main.rs"));
        assert!(!d.should_skip_file("README.md"));
        assert!(!d.should_skip_file("Cargo.toml"));
        assert!(!d.should_skip_file("config.yaml"));
        assert!(!d.should_skip_file("src/config.rs"));
        assert!(!d.should_skip_file(".github/workflows/ci.yml"));
        assert!(!d.should_skip_file("docs/setup.md"));
        // "env.rs" must NOT be skipped — not a dot-env file.
        assert!(!d.should_skip_file("src/env.rs"));
        // "key_manager.rs" must NOT be skipped — extension is "rs", not "key".
        assert!(!d.should_skip_file("src/key_manager.rs"));
    }

    // ── Content-level secret detection tests ──────────────────────────────────

    #[test]
    fn detects_pem_private_key() {
        let d = detector();
        let content =
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----";
        assert!(d.contains_secret(content));
    }

    #[test]
    fn detects_ec_private_key() {
        let d = detector();
        let content = "-----BEGIN EC PRIVATE KEY-----\nMHQCAQEEIO...\n-----END EC PRIVATE KEY-----";
        assert!(d.contains_secret(content));
    }

    #[test]
    fn detects_openssh_private_key() {
        let d = detector();
        let content = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEA...\n-----END OPENSSH PRIVATE KEY-----";
        assert!(d.contains_secret(content));
    }

    /// Broadened PEM header: ENCRYPTED PRIVATE KEY must be detected.
    #[test]
    fn detects_encrypted_private_key_header() {
        let d = detector();
        let content = "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIFHDBOBgkqhkiG9w0...\n-----END ENCRYPTED PRIVATE KEY-----";
        assert!(d.contains_secret(content));
    }

    /// Broadened PEM header: PGP PRIVATE KEY BLOCK must be detected.
    #[test]
    fn detects_pgp_private_key_block() {
        let d = detector();
        let content = "-----BEGIN PGP PRIVATE KEY BLOCK-----\nVersion: GnuPG v1\n...\n-----END PGP PRIVATE KEY BLOCK-----";
        assert!(d.contains_secret(content));
    }

    /// Acceptance criterion: tests must include AKIAIOSFODNN7EXAMPLE and verify it is
    /// detected (would be skipped before embedding in the indexing pipeline).
    #[test]
    fn detects_aws_sample_access_key() {
        let d = detector();
        // The canonical AWS sample key from AWS documentation.
        let content = "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE";
        assert!(
            d.contains_secret(content),
            "AKIAIOSFODNN7EXAMPLE must be detected as an AWS access key"
        );
    }

    #[test]
    fn detects_aws_access_key_in_code() {
        let d = detector();
        let content = r#"
            let key = "AKIAIOSFODNN7EXAMPLE";
            let secret = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
        "#;
        assert!(d.contains_secret(content));
    }

    /// Unquoted dotenv-style secret assignment must be detected (Fix 2 — gap).
    #[test]
    fn detects_unquoted_aws_secret_key() {
        let d = detector();
        // Shell/dotenv style without quotes — was previously missed.
        let content = "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
        assert!(
            d.contains_secret(content),
            "unquoted AWS_SECRET_ACCESS_KEY must be detected"
        );
    }

    /// Unquoted API_KEY assignment must be detected.
    #[test]
    fn detects_unquoted_api_key_assignment() {
        let d = detector();
        let content = "API_KEY=aB3xY9zQ1wE7rT2uP6sD4fG8hJ0k";
        assert!(
            d.contains_secret(content),
            "unquoted API_KEY= must be detected"
        );
    }

    /// Regression: unquoted secret on a NON-FINAL line of a multi-line chunk must be
    /// detected. Without the multiline (`m`) flag, `$` matched only end-of-string, so
    /// mid-chunk unquoted assignments were silently missed.
    ///
    /// The AWS example value used here (`wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY`)
    /// is the canonical AWS documentation example key — safe to use in tests.
    #[test]
    fn detects_unquoted_secret_mid_chunk() {
        let d = detector();

        // Secret on a middle line (not the last line).
        let mid_chunk = "# database config\nAWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\nDB_HOST=localhost\nDEBUG=true\n";
        assert!(
            d.contains_secret(mid_chunk),
            "unquoted secret on a non-final line must be detected (mid-chunk)"
        );

        // Secret on the first line followed by more lines.
        let first_line_chunk = "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\nDB_HOST=localhost\nDEBUG=true";
        assert!(
            d.contains_secret(first_line_chunk),
            "unquoted secret on the first line of a multi-line chunk must be detected"
        );
    }

    #[test]
    fn detects_github_pat_classic() {
        let d = detector();
        // 36-char suffix for classic ghp_ token
        let content = "GITHUB_TOKEN=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghij";
        assert!(d.contains_secret(content));
    }

    #[test]
    fn detects_github_pat_fine_grained() {
        let d = detector();
        // Fine-grained PAT — github_pat_ + 82 chars
        let content = "token: github_pat_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXyz";
        assert!(d.contains_secret(content));
    }

    #[test]
    fn detects_slack_bot_token() {
        let d = detector();
        let content = "SLACK_TOKEN=xoxb-12345678901-12345678901-AbCdEfGhIjKlMnOpQrSt";
        assert!(d.contains_secret(content));
    }

    #[test]
    fn detects_slack_app_token() {
        let d = detector();
        let content = "token = xoxp-12345678901-12345678901-12345678901-abc123def456";
        assert!(d.contains_secret(content));
    }

    /// JWT token must be detected.
    #[test]
    fn detects_jwt_token() {
        let d = detector();
        // Sample JWT (header.payload.signature in base64url).
        let content = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        assert!(d.contains_secret(content), "JWT token must be detected");
    }

    /// Google API key (AIza... 40 chars total) must be detected.
    #[test]
    fn detects_google_api_key() {
        let d = detector();
        // AIza + 35 chars = 39 chars total.
        let content = "const API_KEY = 'AIzaSyD-9tSrke72I6gvXnkdldP123ABC456DEF';";
        assert!(
            d.contains_secret(content),
            "Google API key AIzaSy... must be detected"
        );
    }

    /// Stripe live secret key must be detected.
    #[test]
    fn detects_stripe_secret_key() {
        let d = detector();
        let content = "STRIPE_SECRET_KEY=sk_live_51ABCDefGHIjklMNOpQRsTUVWXYZ01234567";
        assert!(
            d.contains_secret(content),
            "Stripe sk_live_... must be detected"
        );
    }

    /// Stripe test key must be detected.
    #[test]
    fn detects_stripe_test_key() {
        let d = detector();
        let content = r#"stripe_key = "sk_test_4eC39HqLyjWDarjtT1zdp7dc""#;
        assert!(
            d.contains_secret(content),
            "Stripe sk_test_... must be detected"
        );
    }

    /// OpenAI sk-proj-... key must be detected.
    #[test]
    fn detects_openai_project_key() {
        let d = detector();
        let content = "OPENAI_API_KEY=sk-proj-abcdefghijklmnopqrstuvwxyz0123456789ABCDEF";
        assert!(
            d.contains_secret(content),
            "OpenAI sk-proj-... must be detected"
        );
    }

    /// OpenAI legacy sk-... key must be detected.
    #[test]
    fn detects_openai_legacy_key() {
        let d = detector();
        let content = r#"openai_key = "sk-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnop""#;
        assert!(d.contains_secret(content), "OpenAI sk-... must be detected");
    }

    #[test]
    fn detects_high_entropy_secret_assignment() {
        let d = detector();
        // A realistic-looking secret assignment with high entropy value.
        let content = r#"SECRET_KEY = "xK9mP2qRvL8nT5jF3wY7dZ1hU6sA4cB0eGiJoNpQtVuWx""#;
        assert!(d.contains_secret(content));
    }

    #[test]
    fn detects_password_assignment_high_entropy() {
        let d = detector();
        let content = r#"PASSWORD = "Tr0ub4dor&3_super_secure_P@ssw0rd!XYZ""#;
        assert!(d.contains_secret(content));
    }

    /// Mixed-case + digits API_KEY (quoted) must be flagged (positive control for Fix M-1).
    #[test]
    fn detects_api_key_mixed_case_digits() {
        let d = detector();
        let content = r#"API_KEY = "aB3xY9zQ1wE7rT2uP6sD4fG8hJ0kL5mN""#;
        assert!(
            d.contains_secret(content),
            "mixed-case+digit API_KEY must be flagged"
        );
    }

    // ── False-positive / benign tests ─────────────────────────────────────────

    #[test]
    fn benign_short_password_not_flagged() {
        let d = detector();
        // Short, low-entropy value — should NOT trigger.
        let content = r#"password = "hunter2""#;
        assert!(!d.contains_secret(content));
    }

    /// Keyboard walk — high Shannon entropy but single character class (all lowercase).
    /// Must NOT flag after diversity guard (Fix M-1).
    #[test]
    fn benign_keyboard_layout_not_flagged() {
        let d = detector();
        let content = r#"KEYBOARD_LAYOUT = "qwertyuiopasdfghjkl""#;
        assert!(
            !d.contains_secret(content),
            "keyboard walk must not be flagged (single char class)"
        );
    }

    /// Plain English sentence — must NOT flag even if entropy is moderate.
    #[test]
    fn benign_description_not_flagged() {
        let d = detector();
        let content = r#"description = "the quick brown fox jumped over""#;
        assert!(
            !d.contains_secret(content),
            "plain English description must not be flagged"
        );
    }

    /// Long single-class word — must NOT flag.
    #[test]
    fn benign_long_word_not_flagged() {
        let d = detector();
        let content = r#"name = "supercalifragilistic""#;
        assert!(
            !d.contains_secret(content),
            "long single-class word must not be flagged"
        );
    }

    /// AWS region string — must NOT flag.
    #[test]
    fn benign_region_not_flagged() {
        let d = detector();
        let content = r#"region = "us-east-1.compute.internal""#;
        assert!(
            !d.contains_secret(content),
            "AWS region string must not be flagged"
        );
    }

    #[test]
    fn benign_code_not_flagged() {
        let d = detector();
        // Normal source code with no secrets.
        let content = r#"
            fn parse_config(path: &str) -> Result<Config> {
                let file = std::fs::File::open(path)?;
                serde_json::from_reader(file).map_err(Into::into)
            }
        "#;
        assert!(!d.contains_secret(content));
    }

    #[test]
    fn benign_readme_not_flagged() {
        let d = detector();
        let content = r#"
            # Installation
            Run `cargo install vektor --locked` to install.
            Set VEKTOR__EMBEDDING__BACKEND to "onnx" for local mode.
        "#;
        assert!(!d.contains_secret(content));
    }

    #[test]
    fn benign_api_url_not_flagged() {
        let d = detector();
        // An API base URL is not a secret.
        let content = r#"
            const API_URL = "https://api.example.com/v1";
            const MAX_RETRIES: u32 = 3;
        "#;
        assert!(!d.contains_secret(content));
    }

    #[test]
    fn benign_version_string_not_flagged() {
        let d = detector();
        let content = r#"version = "1.2.3""#;
        assert!(!d.contains_secret(content));
    }

    // ── Entropy helper tests ───────────────────────────────────────────────────

    #[test]
    fn entropy_high_for_random_looking_string() {
        // Random base64 strings have high entropy (~5.5 bits/char).
        let entropy = shannon_entropy("xK9mP2qRvL8nT5jF3wY7dZ1hU6sA4cB0eGiJoNpQtVuWxyz");
        assert!(entropy >= ENTROPY_THRESHOLD, "entropy = {entropy:.3}");
    }

    #[test]
    fn entropy_low_for_simple_string() {
        // "aaaaaaaaaa" has entropy 0.
        let entropy = shannon_entropy("aaaaaaaaaa");
        assert!(entropy < ENTROPY_THRESHOLD, "entropy = {entropy:.3}");
    }

    #[test]
    fn entropy_zero_for_empty_string() {
        assert_eq!(shannon_entropy(""), 0.0);
    }
}
