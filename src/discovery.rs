use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::{
    config::Config,
    error::{Result, VektorError},
};

#[allow(dead_code)]
const GENERATED_OR_CACHE_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".next",
    ".nuxt",
    ".pytest_cache",
    ".ruff_cache",
    ".mypy_cache",
    ".tox",
    ".turbo",
    ".venv",
    "__pycache__",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "out",
    "target",
    "venv",
];

#[allow(dead_code)]
pub fn discover_files(root: &Path, config: &Config) -> Result<Vec<PathBuf>> {
    let max_size_bytes = config.index.max_file_size_kb.saturating_mul(1024);
    let mut builder = WalkBuilder::new(root);
    builder
        .standard_filters(true)
        .hidden(false)
        .require_git(false)
        .filter_entry(should_visit_entry);

    let mut files = Vec::new();
    for entry in builder.build() {
        let entry = entry.map_err(ignore_error_to_vektor)?;
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }

        let metadata = entry.metadata().map_err(ignore_error_to_vektor)?;
        if metadata.len() > max_size_bytes {
            tracing::debug!(
                path = %entry.path().display(),
                size_bytes = metadata.len(),
                max_file_size_kb = config.index.max_file_size_kb,
                "skipping oversized file"
            );
            continue;
        }

        files.push(entry.into_path());
    }

    files.sort();
    Ok(files)
}

#[allow(dead_code)]
fn should_visit_entry(entry: &ignore::DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }

    let Some(file_type) = entry.file_type() else {
        return true;
    };
    if !file_type.is_dir() {
        return true;
    }

    entry
        .file_name()
        .to_str()
        .is_none_or(|name| !GENERATED_OR_CACHE_DIRS.contains(&name))
}

#[allow(dead_code)]
fn ignore_error_to_vektor(error: ignore::Error) -> VektorError {
    let message = error.to_string();
    if let Some(io_error) = error.into_io_error() {
        VektorError::Io(io_error)
    } else {
        VektorError::Config(format!("file discovery error: {message}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_files_respects_gitignore_and_nested_gitignore_files() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        write_file(tempdir.path().join(".gitignore"), "ignored.txt\n");
        write_file(tempdir.path().join("kept.rs"), "fn main() {}\n");
        write_file(tempdir.path().join("ignored.txt"), "ignored\n");

        let nested = tempdir.path().join("nested");
        std::fs::create_dir(&nested).expect("create nested dir");
        write_file(nested.join(".gitignore"), "ignored_nested.py\n");
        write_file(nested.join("kept.py"), "print('ok')\n");
        write_file(nested.join("ignored_nested.py"), "print('ignored')\n");

        let files = discover_files(tempdir.path(), &Config::default()).expect("discover files");
        let relative = relative_paths(tempdir.path(), files);

        assert_eq!(
            relative,
            vec![
                ".gitignore",
                "kept.rs",
                "nested/.gitignore",
                "nested/kept.py"
            ]
        );
    }

    #[test]
    fn discover_files_respects_git_info_exclude() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let git_info = tempdir.path().join(".git/info");
        std::fs::create_dir_all(&git_info).expect("create git info dir");
        write_file(git_info.join("exclude"), "excluded.go\n");
        write_file(tempdir.path().join("included.go"), "package main\n");
        write_file(tempdir.path().join("excluded.go"), "package main\n");

        let files = discover_files(tempdir.path(), &Config::default()).expect("discover files");
        let relative = relative_paths(tempdir.path(), files);

        assert_eq!(relative, vec!["included.go"]);
    }

    #[test]
    fn discover_files_skips_generated_cache_dirs_and_large_files() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let mut config = Config::default();
        config.index.max_file_size_kb = 1;

        write_file(tempdir.path().join(".env"), "PHASE2_DISCOVERY=visible\n");
        write_file(
            tempdir.path().join(".github/workflows/ci.yml"),
            "name: ci\n",
        );
        write_file(tempdir.path().join("small.ts"), "export const ok = true;\n");
        write_file(tempdir.path().join("large.ts"), &"x".repeat(1025));
        write_file(
            tempdir.path().join("target/generated.rs"),
            "fn generated() {}\n",
        );
        write_file(
            tempdir.path().join("node_modules/pkg/index.js"),
            "module.exports = {};\n",
        );
        write_file(tempdir.path().join("__pycache__/cached.pyc"), "cached\n");
        write_file(tempdir.path().join("dist/bundle.js"), "bundle\n");
        write_file(tempdir.path().join("build/output.js"), "bundle\n");

        let files = discover_files(tempdir.path(), &config).expect("discover files");
        let relative = relative_paths(tempdir.path(), files);

        assert_eq!(
            relative,
            vec![".env", ".github/workflows/ci.yml", "small.ts"]
        );
    }

    #[test]
    fn discover_files_returns_sorted_paths() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        write_file(tempdir.path().join("z.rs"), "fn z() {}\n");
        write_file(tempdir.path().join("a.rs"), "fn a() {}\n");
        write_file(tempdir.path().join("nested/m.rs"), "fn m() {}\n");

        let files = discover_files(tempdir.path(), &Config::default()).expect("discover files");
        let relative = relative_paths(tempdir.path(), files);

        assert_eq!(relative, vec!["a.rs", "nested/m.rs", "z.rs"]);
    }

    fn write_file(path: impl AsRef<Path>, content: &str) {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, content).expect("write fixture file");
    }

    fn relative_paths(root: &Path, paths: Vec<PathBuf>) -> Vec<String> {
        paths
            .into_iter()
            .map(|path| {
                path.strip_prefix(root)
                    .expect("path is under root")
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect()
    }
}
