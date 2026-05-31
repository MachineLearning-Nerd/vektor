use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::{
    config::Config,
    error::{Result, VektorError},
};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Pending,
    Indexed,
    Failed,
}

impl FileStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Indexed => "indexed",
            Self::Failed => "failed",
        }
    }

    fn from_str(status: &str) -> Result<Self> {
        match status {
            "pending" => Ok(Self::Pending),
            "indexed" => Ok(Self::Indexed),
            "failed" => Ok(Self::Failed),
            _ => Err(VektorError::State(format!(
                "unknown file hash status: {status}"
            ))),
        }
    }
}

#[allow(dead_code)]
pub fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    Ok(hash_bytes_full(&bytes)[..16].to_string())
}

#[allow(dead_code)]
pub fn hash_content(content: &str) -> String {
    hash_bytes_full(content.as_bytes())
}

fn hash_bytes_full(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    hex
}

#[allow(dead_code)]
pub struct HashStore {
    conn: Connection,
    db_path: PathBuf,
}

#[allow(dead_code)]
impl HashStore {
    pub fn open(project_root: &Path, config: &Config) -> Result<Self> {
        let db_path = project_state_db_path(project_root, config)?;
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(&db_path).map_err(sqlite_error)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(sqlite_error)?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS file_hashes (
                rel_path    TEXT PRIMARY KEY,
                hash        TEXT NOT NULL,
                status      TEXT NOT NULL DEFAULT 'pending'
                            CHECK (status IN ('pending', 'indexed', 'failed')),
                indexed_at  INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_file_hashes_status
                ON file_hashes(status);
            "#,
        )
        .map_err(sqlite_error)?;

        Ok(Self { conn, db_path })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn get_hash(&self, rel_path: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT hash FROM file_hashes WHERE rel_path = ?1",
                params![rel_path],
                |row| row.get(0),
            )
            .optional()
            .map_err(sqlite_error)
    }

    pub fn get_status(&self, rel_path: &str) -> Result<Option<FileStatus>> {
        let status = self
            .conn
            .query_row(
                "SELECT status FROM file_hashes WHERE rel_path = ?1",
                params![rel_path],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(sqlite_error)?;

        status.as_deref().map(FileStatus::from_str).transpose()
    }

    pub fn set_hash(&self, rel_path: &str, hash: &str, status: FileStatus) -> Result<()> {
        let indexed_at = match status {
            FileStatus::Pending => None,
            FileStatus::Indexed | FileStatus::Failed => Some(current_unix_epoch()?),
        };

        self.conn
            .execute(
                r#"
                INSERT INTO file_hashes (rel_path, hash, status, indexed_at)
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(rel_path) DO UPDATE SET
                    hash = excluded.hash,
                    status = excluded.status,
                    indexed_at = excluded.indexed_at
                "#,
                params![rel_path, hash, status.as_str(), indexed_at],
            )
            .map_err(sqlite_error)?;

        Ok(())
    }

    pub fn is_changed(&self, path: &Path, root: &Path) -> Result<bool> {
        let rel_path = relative_path_string(path, root)?;
        let current_hash = hash_file(path)?;

        Ok(self.get_hash(&rel_path)?.as_deref() != Some(current_hash.as_str()))
    }

    pub fn get_pending(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT rel_path FROM file_hashes WHERE status = 'pending' ORDER BY rel_path")
            .map_err(sqlite_error)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sqlite_error)?;

        let mut pending = Vec::new();
        for row in rows {
            pending.push(row.map_err(sqlite_error)?);
        }

        Ok(pending)
    }
}

fn project_state_db_path(project_root: &Path, config: &Config) -> Result<PathBuf> {
    Ok(project_data_dir(project_root, config)?.join("state.db"))
}

/// Resolve the project-scoped data directory `<data_dir>/<project-hash>/`.
///
/// The project hash is a SHA-256 of the canonicalized project root, so every
/// per-project artifact (`state.db`, the LanceDB `lance/` dir, sidecar
/// metadata) colocates under the same directory. Shared with `vector_store`.
pub(crate) fn project_data_dir(project_root: &Path, config: &Config) -> Result<PathBuf> {
    let canonical_root = project_root.canonicalize()?;
    let project_key = hash_content(&canonical_root.to_string_lossy());
    Ok(expand_data_dir(&config.index.data_dir)?.join(project_key))
}

/// Expand a configured `data_dir` string into an absolute path, resolving a
/// leading `~` / `~/` / `~\` to the user's home directory.
///
/// Shared with `embedder::onnx` so model-artifact resolution uses the exact
/// same home-expansion rule as project state — never duplicate this logic.
pub(crate) fn expand_data_dir(data_dir: &str) -> Result<PathBuf> {
    if data_dir == "~" {
        return dirs::home_dir()
            .ok_or_else(|| VektorError::Config("home directory not found".into()));
    }

    if let Some(rest) = data_dir
        .strip_prefix("~/")
        .or_else(|| data_dir.strip_prefix("~\\"))
    {
        let home = dirs::home_dir()
            .ok_or_else(|| VektorError::Config("home directory not found".into()))?;
        return Ok(home.join(rest));
    }

    Ok(PathBuf::from(data_dir))
}

fn relative_path_string(path: &Path, root: &Path) -> Result<String> {
    let rel_path = path.strip_prefix(root).map_err(|error| {
        VektorError::State(format!(
            "path {} is not under root {}: {error}",
            path.display(),
            root.display()
        ))
    })?;

    Ok(rel_path.to_string_lossy().replace('\\', "/"))
}

fn current_unix_epoch() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| VektorError::State(format!("system clock before Unix epoch: {error}")))?;

    i64::try_from(duration.as_secs())
        .map_err(|error| VektorError::State(format!("timestamp overflow: {error}")))
}

fn sqlite_error(error: rusqlite::Error) -> VektorError {
    VektorError::State(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_store_hash_file_returns_truncated_sha256_and_full_content_hash() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let path = tempdir.path().join("hello.txt");
        std::fs::write(&path, "hello\n").expect("write file");

        assert_eq!(hash_file(&path).expect("hash file"), "5891b5b522d5df08");
        assert_eq!(
            hash_content("hello\n"),
            "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"
        );
    }

    #[test]
    fn hash_store_creates_project_scoped_state_db_under_expanded_data_dir() {
        let fake_home = tempfile::tempdir().expect("create fake home");
        let project = tempfile::tempdir().expect("create project");
        let config = Config {
            index: crate::config::IndexConfig {
                data_dir: "~/.vektor-test".into(),
                ..Default::default()
            },
            ..Default::default()
        };

        temp_env::with_vars(isolated_home(fake_home.path()), || {
            let store = HashStore::open(project.path(), &config).expect("open hash store");
            let db_path = store.db_path();

            assert!(db_path.starts_with(fake_home.path().join(".vektor-test")));
            assert!(db_path.ends_with("state.db"));
            assert!(db_path.exists());
            assert_ne!(db_path.parent(), Some(fake_home.path()));
        });
    }

    #[test]
    fn hash_store_get_set_is_changed_and_status_transitions() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let project = tempdir.path().join("project");
        std::fs::create_dir_all(project.join("src")).expect("create project");
        let file_path = project.join("src/lib.rs");
        std::fs::write(&file_path, "pub fn old() {}\n").expect("write source");

        let config = config_with_data_dir(tempdir.path().join("data"));
        let store = HashStore::open(&project, &config).expect("open hash store");
        let hash = hash_file(&file_path).expect("hash file");

        assert!(store.is_changed(&file_path, &project).expect("is changed"));

        store
            .set_hash("src/lib.rs", &hash, FileStatus::Pending)
            .expect("set pending hash");
        assert_eq!(
            store.get_hash("src/lib.rs").expect("get hash"),
            Some(hash.clone())
        );
        assert_eq!(
            store.get_status("src/lib.rs").expect("get status"),
            Some(FileStatus::Pending)
        );
        assert_eq!(
            store.get_pending().expect("get pending"),
            vec!["src/lib.rs"]
        );

        store
            .set_hash("src/lib.rs", &hash, FileStatus::Indexed)
            .expect("set indexed hash");
        assert_eq!(
            store.get_status("src/lib.rs").expect("get status"),
            Some(FileStatus::Indexed)
        );
        assert!(!store.is_changed(&file_path, &project).expect("is changed"));
        assert!(store.get_pending().expect("get pending").is_empty());

        std::fs::write(&file_path, "pub fn new() {}\n").expect("modify source");
        assert!(store.is_changed(&file_path, &project).expect("is changed"));

        let new_hash = hash_file(&file_path).expect("hash modified file");
        store
            .set_hash("src/lib.rs", &new_hash, FileStatus::Failed)
            .expect("set failed hash");
        assert_eq!(
            store.get_status("src/lib.rs").expect("get status"),
            Some(FileStatus::Failed)
        );
    }

    #[test]
    fn hash_store_get_pending_returns_paths_in_order() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let config = config_with_data_dir(tempdir.path().join("data"));
        let store = HashStore::open(tempdir.path(), &config).expect("open hash store");

        store
            .set_hash("z.rs", "hash-z", FileStatus::Pending)
            .expect("set z");
        store
            .set_hash("a.rs", "hash-a", FileStatus::Pending)
            .expect("set a");
        store
            .set_hash("m.rs", "hash-m", FileStatus::Indexed)
            .expect("set m");

        assert_eq!(
            store.get_pending().expect("get pending"),
            vec!["a.rs", "z.rs"]
        );
    }

    fn config_with_data_dir(data_dir: PathBuf) -> Config {
        Config {
            index: crate::config::IndexConfig {
                data_dir: data_dir.to_string_lossy().into_owned(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn isolated_home(fake_home: &Path) -> Vec<(&'static str, Option<String>)> {
        let home = fake_home.to_string_lossy().into_owned();
        vec![("HOME", Some(home.clone())), ("USERPROFILE", Some(home))]
    }
}
