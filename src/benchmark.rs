use std::{
    collections::HashSet,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use uuid::Uuid;

use crate::{
    chunker::chunk_file,
    config::{Config, IndexConfig},
    error::{Result, VektorError},
    text_index::TextIndex,
};

const CHECKED_IN_FIXTURE: &str = include_str!("../benches/fixtures/tokio_20_queries.json");
const BENCHMARK_TOP_K: usize = 5;
const BENCHMARK_CANDIDATE_LIMIT: usize = BENCHMARK_TOP_K * 4;
const EXPECTED_QUERY_COUNT: usize = 20;

#[derive(Debug, Clone, Deserialize)]
pub struct BenchmarkFixture {
    corpus: Vec<FixtureFile>,
    queries: Vec<LabeledQuery>,
}

#[derive(Debug, Clone, Deserialize)]
struct FixtureFile {
    path: String,
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LabeledQuery {
    id: String,
    query: String,
    relevant_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenchmarkReport {
    pub query_count: usize,
    pub precision_at_5: f64,
    pub recall_at_5: f64,
    pub mrr: f64,
    pub results: Vec<QueryBenchmarkResult>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryBenchmarkResult {
    pub id: String,
    pub query: String,
    pub expected: Vec<String>,
    pub actual: Vec<String>,
    pub precision_at_5: f64,
    pub recall_at_5: f64,
    pub reciprocal_rank: f64,
}

impl BenchmarkReport {
    pub fn render_markdown(&self) -> String {
        let mut output = String::new();
        writeln!(&mut output, "# Vektor retrieval benchmark").expect("write String");
        writeln!(&mut output).expect("write String");
        writeln!(&mut output, "- Queries: {}", self.query_count).expect("write String");
        writeln!(&mut output, "- Precision@5: {:.3}", self.precision_at_5).expect("write String");
        writeln!(&mut output, "- Recall@5: {:.3}", self.recall_at_5).expect("write String");
        writeln!(&mut output, "- MRR: {:.3}", self.mrr).expect("write String");
        output
    }
}

pub fn run_checked_in_benchmark() -> Result<BenchmarkReport> {
    let fixture = checked_in_fixture()?;
    validate_fixture(&fixture)?;
    run_fixture_benchmark(&fixture)
}

pub fn checked_in_fixture() -> Result<BenchmarkFixture> {
    serde_json::from_str(CHECKED_IN_FIXTURE).map_err(|error| {
        VektorError::Config(format!(
            "failed to parse checked-in tokio benchmark fixture: {error}"
        ))
    })
}

fn validate_fixture(fixture: &BenchmarkFixture) -> Result<()> {
    if fixture.queries.len() != EXPECTED_QUERY_COUNT {
        return Err(VektorError::Config(format!(
            "tokio benchmark fixture must contain exactly {EXPECTED_QUERY_COUNT} queries, found {}",
            fixture.queries.len()
        )));
    }

    let corpus_paths = fixture
        .corpus
        .iter()
        .map(|file| file.path.as_str())
        .collect::<HashSet<_>>();
    for query in &fixture.queries {
        if query.relevant_paths.is_empty() {
            return Err(VektorError::Config(format!(
                "benchmark query {} has no relevant paths",
                query.id
            )));
        }
        for path in &query.relevant_paths {
            if !corpus_paths.contains(path.as_str()) {
                return Err(VektorError::Config(format!(
                    "benchmark query {} references missing corpus path {}",
                    query.id, path
                )));
            }
        }
    }

    Ok(())
}

fn run_fixture_benchmark(fixture: &BenchmarkFixture) -> Result<BenchmarkReport> {
    let project = TempProject::new()?;
    write_fixture_corpus(project.root(), &fixture.corpus)?;
    let config = benchmark_config(&project.state_dir());
    let mut text_index = TextIndex::new(project.root(), &config)?;

    for file in &fixture.corpus {
        let chunks = chunk_file(Path::new(&file.path), &file.content, &config);
        text_index.add_chunks(&chunks)?;
    }
    text_index.commit()?;

    let mut results = Vec::with_capacity(fixture.queries.len());
    for query in &fixture.queries {
        let hits = text_index.search(&query.query, BENCHMARK_CANDIDATE_LIMIT)?;
        let mut seen_paths = HashSet::new();
        let actual = hits
            .into_iter()
            .map(|hit| hit.rel_path)
            .filter(|path| seen_paths.insert(path.clone()))
            .take(BENCHMARK_TOP_K)
            .collect::<Vec<_>>();
        results.push(score_query(query, actual));
    }

    Ok(aggregate_results(results))
}

fn write_fixture_corpus(root: &Path, corpus: &[FixtureFile]) -> Result<()> {
    for file in corpus {
        let path = root.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, &file.content)?;
    }
    Ok(())
}

fn benchmark_config(state_dir: &Path) -> Config {
    Config {
        index: IndexConfig {
            data_dir: state_dir.to_string_lossy().into_owned(),
            max_file_size_kb: 1024,
            chunk_max_lines: 80,
            chunk_overlap_pct: 25,
            doc_chunk_max_lines: 40,
        },
        ..Default::default()
    }
}

fn score_query(query: &LabeledQuery, actual: Vec<String>) -> QueryBenchmarkResult {
    let expected = query
        .relevant_paths
        .iter()
        .cloned()
        .collect::<HashSet<String>>();
    let relevant_hits = actual
        .iter()
        .take(BENCHMARK_TOP_K)
        .filter(|path| expected.contains(*path))
        .count();
    let first_relevant_rank = actual
        .iter()
        .take(BENCHMARK_TOP_K)
        .position(|path| expected.contains(path))
        .map(|index| index + 1);

    QueryBenchmarkResult {
        id: query.id.clone(),
        query: query.query.clone(),
        expected: query.relevant_paths.clone(),
        actual,
        precision_at_5: relevant_hits as f64 / BENCHMARK_TOP_K as f64,
        recall_at_5: relevant_hits as f64 / expected.len() as f64,
        reciprocal_rank: first_relevant_rank
            .map(|rank| 1.0 / rank as f64)
            .unwrap_or(0.0),
    }
}

fn aggregate_results(results: Vec<QueryBenchmarkResult>) -> BenchmarkReport {
    let query_count = results.len();
    let denominator = query_count.max(1) as f64;
    let precision_at_5 = results
        .iter()
        .map(|result| result.precision_at_5)
        .sum::<f64>()
        / denominator;
    let recall_at_5 = results.iter().map(|result| result.recall_at_5).sum::<f64>() / denominator;
    let mrr = results
        .iter()
        .map(|result| result.reciprocal_rank)
        .sum::<f64>()
        / denominator;

    BenchmarkReport {
        query_count,
        precision_at_5,
        recall_at_5,
        mrr,
        results,
    }
}

struct TempProject {
    root: PathBuf,
}

impl TempProject {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir().join(format!("vektor-benchmark-{}", Uuid::new_v4()));
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn state_dir(&self) -> PathBuf {
        self.root.join(".vektor-state")
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_fixture_has_twenty_labeled_queries() {
        let fixture = checked_in_fixture().expect("fixture parses");
        validate_fixture(&fixture).expect("fixture is valid");
        assert_eq!(fixture.queries.len(), EXPECTED_QUERY_COUNT);
    }

    #[test]
    fn benchmark_metrics_compute_precision_recall_and_mrr() {
        let query = LabeledQuery {
            id: "mpsc".into(),
            query: "bounded sender receiver".into(),
            relevant_paths: vec!["tokio/src/sync/mpsc/bounded.rs".into()],
        };
        let result = score_query(
            &query,
            vec![
                "tokio/src/runtime/builder.rs".into(),
                "tokio/src/sync/mpsc/bounded.rs".into(),
            ],
        );

        assert_eq!(result.precision_at_5, 0.2);
        assert_eq!(result.recall_at_5, 1.0);
        assert_eq!(result.reciprocal_rank, 0.5);
    }

    #[test]
    fn benchmark_runner_emits_quality_metrics() {
        let report = run_checked_in_benchmark().expect("benchmark runs");

        assert_eq!(report.query_count, EXPECTED_QUERY_COUNT);
        assert!(report.precision_at_5 > 0.0);
        assert!(report.recall_at_5 > 0.0);
        assert!(report.mrr > 0.0);
        assert!(report.render_markdown().contains("Precision@5"));
    }
}
