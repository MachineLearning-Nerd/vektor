use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    chunker::{Chunk, detect_language},
    config::Config,
    discovery::discover_files,
    error::{Result, VektorError},
    state::hash_content,
    text_index::TextIndex,
};

#[allow(dead_code)]
const HEAD_LINES: usize = 50;
#[allow(dead_code)]
const TAIL_LINES: usize = 20;

#[allow(dead_code)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ShallowStats {
    pub(crate) files: usize,
    pub(crate) docs: usize,
    pub(crate) failed: usize,
}

#[allow(dead_code)]
pub(crate) struct ShallowIndexer;

#[allow(dead_code)]
impl ShallowIndexer {
    pub(crate) fn build(path: &Path, config: &Config) -> Result<ShallowStats> {
        let (root, files) = collect_shallow_files(path, config)?;
        let mut text_index = TextIndex::new(&root, config)?;
        let mut stats = ShallowStats::default();

        for file in files {
            stats.files += 1;

            let content = match read_file_lossy(&file.path) {
                Ok(content) => content,
                Err(error) => {
                    stats.failed += 1;
                    tracing::debug!(
                        path = %file.path.display(),
                        error = %error,
                        "failed to read file during shallow index"
                    );
                    continue;
                }
            };
            let projection = shallow_projection(&file.rel_path, &content);
            let chunk = shallow_chunk(&file.rel_path, projection);

            text_index.delete_by_file(&file.rel_path)?;
            text_index.add_shallow_chunks(std::slice::from_ref(&chunk))?;
            stats.docs += 1;
        }

        text_index.commit_with_ready_marker(false)?;
        Ok(stats)
    }
}

#[allow(dead_code)]
#[derive(Debug)]
struct ShallowFile {
    path: PathBuf,
    rel_path: String,
}

#[allow(dead_code)]
fn collect_shallow_files(path: &Path, config: &Config) -> Result<(PathBuf, Vec<ShallowFile>)> {
    if path.is_file() {
        let root = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        if file_is_oversized(path, config)? {
            return Ok((root, Vec::new()));
        }
        let rel_path = relative_path(&root, path);
        return Ok((
            root,
            vec![ShallowFile {
                path: path.to_path_buf(),
                rel_path,
            }],
        ));
    }

    if !path.is_dir() {
        return Err(VektorError::Config(format!(
            "index path does not exist or is not readable: {}",
            path.display()
        )));
    }

    let root = path.to_path_buf();
    let files = discover_files(&root, config)?
        .into_iter()
        .map(|file| {
            let rel_path = relative_path(&root, &file);
            ShallowFile {
                path: file,
                rel_path,
            }
        })
        .collect();

    Ok((root, files))
}

#[allow(dead_code)]
fn file_is_oversized(path: &Path, config: &Config) -> Result<bool> {
    let max_size_bytes = config.index.max_file_size_kb.saturating_mul(1024);
    Ok(fs::metadata(path)?.len() > max_size_bytes)
}

#[allow(dead_code)]
fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[allow(dead_code)]
fn read_file_lossy(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[allow(dead_code)]
fn shallow_chunk(rel_path: &str, content: String) -> Chunk {
    let end_line = content.lines().count().max(1);
    let content_hash = hash_content(&content);
    let id = hash_content(&format!("shallow:{rel_path}:{content_hash}"));

    Chunk {
        id,
        content,
        content_hash,
        rel_path: rel_path.to_string(),
        start_line: 1,
        end_line,
        symbol_name: None,
        symbol_type: None,
        language: detect_language(Path::new(rel_path)),
    }
}

#[allow(dead_code)]
fn shallow_projection(rel_path: &str, content: &str) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    let mut selected = BTreeSet::new();

    for index in 0..lines.len().min(HEAD_LINES) {
        selected.insert(index);
    }
    for index in lines.len().saturating_sub(TAIL_LINES)..lines.len() {
        selected.insert(index);
    }
    for (index, line) in lines.iter().enumerate() {
        if is_declaration_line(line) {
            selected.insert(index);
        }
    }

    let mut projection = String::new();
    projection.push_str("path: ");
    projection.push_str(rel_path);
    projection.push('\n');
    for index in selected {
        projection.push_str(lines[index]);
        projection.push('\n');
    }
    projection
}

#[allow(dead_code)]
fn is_declaration_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    const DECLARATION_PREFIXES: &[&str] = &[
        "async fn ",
        "class ",
        "const ",
        "def ",
        "enum ",
        "export ",
        "fn ",
        "func ",
        "impl ",
        "interface ",
        "private ",
        "protected ",
        "pub async fn ",
        "pub const ",
        "pub enum ",
        "pub fn ",
        "pub struct ",
        "pub trait ",
        "public ",
        "struct ",
        "trait ",
        "type ",
    ];
    DECLARATION_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        text_index::{DEEP_INDEX_DEPTH, SHALLOW_INDEX_DEPTH, TextIndex},
    };
    use tantivy::{Term, collector::Count, query::TermQuery, schema::IndexRecordOption};

    fn config_for(data_dir: &std::path::Path) -> Config {
        let mut config = Config::default();
        config.index.data_dir = data_dir.to_string_lossy().into_owned();
        config
    }

    #[test]
    fn shallow_content_does_not_duplicate_short_files() {
        let content = (1..=40)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");

        let shallow = shallow_projection("src/lib.rs", &content);

        assert_eq!(shallow.lines().filter(|line| *line == "line 1").count(), 1);
        assert_eq!(shallow.lines().filter(|line| *line == "line 40").count(), 1);
    }

    #[test]
    fn shallow_content_includes_declarations_outside_head_and_tail() {
        let mut lines = (1..=120)
            .map(|line| format!("// filler {line}"))
            .collect::<Vec<_>>();
        lines[80] = "pub fn middle_declaration_needle() -> bool { true }".to_string();
        let content = lines.join("\n");

        let shallow = shallow_projection("src/lib.rs", &content);

        assert!(shallow.contains("src/lib.rs"));
        assert!(shallow.contains("middle_declaration_needle"));
    }

    #[tokio::test]
    async fn build_writes_searchable_shallow_docs() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).expect("mkdir src");
        std::fs::write(
            repo.join("src/lib.rs"),
            "pub fn shallowneedle() -> bool {\n    true\n}\n",
        )
        .expect("write lib");
        let config = config_for(&tempdir.path().join("data"));

        let stats = ShallowIndexer::build(&repo, &config).expect("build shallow index");
        let index = TextIndex::open_readonly(&repo, &config).expect("open shallow index");
        let hits = index.search("shallowneedle", 5).expect("search shallow");

        assert_eq!(stats.files, 1);
        assert_eq!(stats.docs, 1);
        assert_eq!(hits[0].rel_path, "src/lib.rs");
        assert_eq!(count_depth(&index, SHALLOW_INDEX_DEPTH), 1);
        assert_eq!(count_depth(&index, DEEP_INDEX_DEPTH), 0);
    }

    #[tokio::test]
    async fn deep_delete_by_file_removes_shallow_docs() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).expect("mkdir src");
        std::fs::write(
            repo.join("src/lib.rs"),
            "pub fn removeshallowneedle() -> bool {\n    true\n}\n",
        )
        .expect("write lib");
        let config = config_for(&tempdir.path().join("data"));

        ShallowIndexer::build(&repo, &config).expect("build shallow index");
        let mut index = TextIndex::new(&repo, &config).expect("open writer");
        index
            .delete_by_file("src/lib.rs")
            .expect("delete shallow docs");
        index.commit().expect("commit delete");
        let hits = index.search("removeshallowneedle", 5).expect("search");

        assert!(hits.is_empty());
    }

    #[test]
    fn collect_shallow_files_for_single_file_keeps_nested_relative_path() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo = tempdir.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).expect("mkdir src");
        let file = repo.join("src").join("main.rs");
        std::fs::write(&file, "fn main() {}\n").expect("write main");
        let config = config_for(&tempdir.path().join("data"));

        let (root, files) = collect_shallow_files(&file, &config).expect("collect single file");

        assert_eq!(root, repo.join("src"));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].rel_path, "main.rs");
    }

    fn count_depth(index: &TextIndex, depth: &str) -> usize {
        let term = Term::from_field_text(index.fields().index_depth, depth);
        let query = TermQuery::new(term, IndexRecordOption::Basic);
        index
            .reader()
            .searcher()
            .search(&query, &Count)
            .expect("count depth")
    }
}
