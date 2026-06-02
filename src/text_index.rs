//! Tantivy full-text index schema and project-scoped open/create.
//!
//! This module owns the Phase 4 keyword-search storage contract only:
//! schema construction, field handles, idempotent index opening, and batched
//! chunk writes. BM25 querying is handled by later Phase 4 tasks.

use std::{
    fs,
    path::{Path, PathBuf},
};

use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy, Score, TantivyDocument, Term,
    collector::TopDocs,
    directory::MmapDirectory,
    doc,
    query::QueryParser,
    schema::{
        Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions,
        Value as _,
    },
};

use crate::{
    chunker::Chunk,
    config::Config,
    error::{Result, VektorError},
    state::project_data_dir,
};

const TANTIVY_SUBDIR: &str = "tantivy";
const EN_STEM_TOKENIZER: &str = "en_stem";
const TANTIVY_WRITER_HEAP_BYTES: usize = 50 * 1024 * 1024;
const PHASE_4_INDEX_DEPTH: &str = "deep";
const SYMBOL_NAME_FIELD_BOOST: Score = 2.0;

/// One BM25 keyword hit returned by [`TextIndex::search`].
///
/// `#[allow(dead_code)]`: consumed by Phase 4 hybrid fusion (4.5) and MCP
/// handlers after this task; tests cover the storage projection now.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct KeywordHit {
    pub(crate) chunk_id: String,
    pub(crate) rel_path: String,
    pub(crate) score: Score,
    pub(crate) start_line: u64,
    pub(crate) end_line: u64,
    pub(crate) symbol_name: Option<String>,
    pub(crate) language: String,
}

/// Tantivy field handles for the text-index schema.
///
/// Retained alongside the schema so the later add/search tasks can address
/// fields without string lookup at every call site.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextIndexFields {
    pub(crate) chunk_id: Field,
    pub(crate) rel_path: Field,
    pub(crate) content: Field,
    pub(crate) symbol_name: Field,
    pub(crate) language: Field,
    pub(crate) start_line: Field,
    pub(crate) end_line: Field,
    pub(crate) index_depth: Field,
}

/// Project-scoped Tantivy text index.
#[allow(dead_code)]
pub(crate) struct TextIndex {
    index: Index,
    writer: IndexWriter<TantivyDocument>,
    reader: IndexReader,
    schema: Schema,
    fields: TextIndexFields,
    tantivy_dir: PathBuf,
}

#[allow(dead_code)]
impl TextIndex {
    /// Open (or create) the project-scoped Tantivy index.
    ///
    /// The index lives at `<data_dir>/<project-hash>/tantivy/`, next to
    /// `VectorStore`'s LanceDB directory and the hash-state database.
    pub(crate) fn new(project_root: &Path, config: &Config) -> Result<Self> {
        let project_dir = project_data_dir(project_root, config)?;
        let tantivy_dir = project_dir.join(TANTIVY_SUBDIR);
        fs::create_dir_all(&tantivy_dir)?;

        let (schema, fields) = build_schema();
        let directory = MmapDirectory::open(&tantivy_dir).map_err(|error| {
            VektorError::Storage(format!(
                "failed to open Tantivy directory {}: {error}",
                tantivy_dir.display()
            ))
        })?;
        let index = Index::open_or_create(directory, schema.clone()).map_err(|error| {
            VektorError::Storage(format!(
                "failed to open or create Tantivy index {}: {error}",
                tantivy_dir.display()
            ))
        })?;

        ensure_en_stem_tokenizer(&index)?;

        let writer = index
            .writer_with_num_threads(1, TANTIVY_WRITER_HEAP_BYTES)
            .map_err(|error| {
                tantivy_storage_error("failed to create Tantivy index writer", error)
            })?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|error| {
                tantivy_storage_error("failed to create Tantivy index reader", error)
            })?;

        Ok(Self {
            index,
            writer,
            reader,
            schema,
            fields,
            tantivy_dir,
        })
    }

    pub(crate) fn index(&self) -> &Index {
        &self.index
    }

    pub(crate) fn reader(&self) -> &IndexReader {
        &self.reader
    }

    pub(crate) fn schema(&self) -> &Schema {
        &self.schema
    }

    pub(crate) fn fields(&self) -> &TextIndexFields {
        &self.fields
    }

    pub(crate) fn tantivy_dir(&self) -> &Path {
        &self.tantivy_dir
    }

    /// Add chunks to the pending Tantivy batch without committing.
    ///
    /// The caller owns chunk-id generation; this method stores `Chunk::id`
    /// directly so Tantivy rows join with the LanceDB rows for the same chunk.
    pub(crate) fn add_chunks(&mut self, chunks: &[Chunk]) -> Result<()> {
        for chunk in chunks {
            let start_line = u64::try_from(chunk.start_line).map_err(|_| {
                VektorError::Storage(format!(
                    "chunk {} start_line {} does not fit in Tantivy u64 field",
                    chunk.id, chunk.start_line
                ))
            })?;
            let end_line = u64::try_from(chunk.end_line).map_err(|_| {
                VektorError::Storage(format!(
                    "chunk {} end_line {} does not fit in Tantivy u64 field",
                    chunk.id, chunk.end_line
                ))
            })?;
            let symbol_name = chunk.symbol_name.as_deref().unwrap_or("");
            let language = chunk
                .language
                .map(crate::chunker::Language::as_str)
                .unwrap_or("unknown");

            self.writer
                .add_document(doc!(
                    self.fields.chunk_id => chunk.id.as_str(),
                    self.fields.rel_path => chunk.rel_path.as_str(),
                    self.fields.content => chunk.content.as_str(),
                    self.fields.symbol_name => symbol_name,
                    self.fields.language => language,
                    self.fields.start_line => start_line,
                    self.fields.end_line => end_line,
                    self.fields.index_depth => PHASE_4_INDEX_DEPTH,
                ))
                .map_err(|error| {
                    tantivy_storage_error(
                        &format!("failed to add chunk {} to Tantivy batch", chunk.id),
                        error,
                    )
                })?;
        }

        Ok(())
    }

    /// Queue deletion of all Tantivy docs for an exact repository-relative path.
    pub(crate) fn delete_by_file(&mut self, rel_path: &str) -> Result<()> {
        self.writer
            .delete_term(Term::from_field_text(self.fields.rel_path, rel_path));
        Ok(())
    }

    /// Search the Tantivy index with BM25 over content and boosted symbols.
    ///
    /// `symbol_name` receives the PRD §4.10 2.0x boost. Scores are raw BM25
    /// scores from Tantivy; downstream RRF fusion consumes only rank order.
    pub(crate) fn search(&self, query: &str, top_k: usize) -> Result<Vec<KeywordHit>> {
        let query = query.trim();
        if query.is_empty() || top_k == 0 {
            return Ok(Vec::new());
        }

        let mut query_parser = QueryParser::for_index(
            &self.index,
            vec![self.fields.content, self.fields.symbol_name],
        );
        query_parser.set_field_boost(self.fields.symbol_name, SYMBOL_NAME_FIELD_BOOST);
        let query = query_parser
            .parse_query(query)
            .map_err(|error| VektorError::Parse(format!("failed to parse BM25 query: {error}")))?;

        let searcher = self.reader.searcher();
        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(top_k).order_by_score())
            .map_err(|error| tantivy_storage_error("failed to search Tantivy index", error))?;

        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, doc_address) in top_docs {
            let doc = searcher.doc(doc_address).map_err(|error| {
                tantivy_storage_error("failed to load Tantivy search hit document", error)
            })?;
            hits.push(keyword_hit_from_doc(&doc, score, self.fields)?);
        }

        hits.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.chunk_id.cmp(&right.chunk_id))
        });

        Ok(hits)
    }

    /// Commit the pending Tantivy batch and reload the manual reader generation.
    pub(crate) fn commit(&mut self) -> Result<()> {
        self.writer.commit().map_err(|error| {
            tantivy_storage_error("failed to commit Tantivy index batch", error)
        })?;
        self.reader.reload().map_err(|error| {
            tantivy_storage_error("failed to reload Tantivy index reader", error)
        })?;
        Ok(())
    }
}

fn keyword_hit_from_doc(
    doc: &TantivyDocument,
    score: Score,
    fields: TextIndexFields,
) -> Result<KeywordHit> {
    let symbol_name = stored_text(doc, fields.symbol_name, "symbol_name")?;
    let symbol_name = if symbol_name.is_empty() {
        None
    } else {
        Some(symbol_name)
    };

    Ok(KeywordHit {
        chunk_id: stored_text(doc, fields.chunk_id, "chunk_id")?,
        rel_path: stored_text(doc, fields.rel_path, "rel_path")?,
        score,
        start_line: stored_u64(doc, fields.start_line, "start_line")?,
        end_line: stored_u64(doc, fields.end_line, "end_line")?,
        symbol_name,
        language: stored_text(doc, fields.language, "language")?,
    })
}

fn stored_text(doc: &TantivyDocument, field: Field, field_name: &str) -> Result<String> {
    doc.get_first(field)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            VektorError::Storage(format!(
                "Tantivy search result missing stored text field `{field_name}`"
            ))
        })
}

fn stored_u64(doc: &TantivyDocument, field: Field, field_name: &str) -> Result<u64> {
    doc.get_first(field)
        .and_then(|value| value.as_u64())
        .ok_or_else(|| {
            VektorError::Storage(format!(
                "Tantivy search result missing stored u64 field `{field_name}`"
            ))
        })
}

fn build_schema() -> (Schema, TextIndexFields) {
    let mut builder = Schema::builder();

    let chunk_id = builder.add_text_field("chunk_id", STORED);
    let rel_path = builder.add_text_field("rel_path", STRING | STORED);
    let content = builder.add_text_field("content", stemmed_text_options());
    let symbol_name = builder.add_text_field("symbol_name", stemmed_text_options().set_stored());
    let language = builder.add_text_field("language", STRING | STORED);
    let start_line = builder.add_u64_field("start_line", STORED);
    let end_line = builder.add_u64_field("end_line", STORED);
    let index_depth = builder.add_text_field("index_depth", STRING | STORED);

    let schema = builder.build();
    let fields = TextIndexFields {
        chunk_id,
        rel_path,
        content,
        symbol_name,
        language,
        start_line,
        end_line,
        index_depth,
    };

    (schema, fields)
}

fn stemmed_text_options() -> TextOptions {
    TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(EN_STEM_TOKENIZER)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    )
}

fn ensure_en_stem_tokenizer(index: &Index) -> Result<()> {
    if index.tokenizers().get(EN_STEM_TOKENIZER).is_some() {
        return Ok(());
    }

    let default_tokenizers = tantivy::tokenizer::TokenizerManager::default();
    let tokenizer = default_tokenizers.get(EN_STEM_TOKENIZER).ok_or_else(|| {
        VektorError::Storage(format!(
            "Tantivy tokenizer `{EN_STEM_TOKENIZER}` is unavailable; enable Tantivy stemmer support"
        ))
    })?;
    index.tokenizers().register(EN_STEM_TOKENIZER, tokenizer);

    Ok(())
}

fn tantivy_storage_error(context: &str, error: tantivy::TantivyError) -> VektorError {
    VektorError::Storage(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tantivy::{
        collector::{Count, TopDocs},
        query::QueryParser,
        schema::{FieldType, NumericOptions, Value as _},
    };

    use super::*;
    use crate::{
        chunker::{Chunk, Language},
        vector_store::{ChunkRow, VectorStore},
    };

    const SEARCH_DIM: usize = 4;
    const SEARCH_MODEL: &str = "test-model-4d";

    struct TextIndexFixture {
        _project: tempfile::TempDir,
        _data_dir: tempfile::TempDir,
        index: TextIndex,
    }

    fn config_with_data_dir(data_dir: &Path) -> Config {
        Config {
            index: crate::config::IndexConfig {
                data_dir: data_dir.to_string_lossy().into_owned(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn text_index_fixture() -> TextIndexFixture {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());
        let index = TextIndex::new(project.path(), &config).expect("create text index");

        TextIndexFixture {
            _project: project,
            _data_dir: data_dir,
            index,
        }
    }

    fn chunk(
        id: &str,
        rel_path: &str,
        content: &str,
        symbol_name: Option<&str>,
        language: Option<Language>,
        start_line: usize,
        end_line: usize,
    ) -> Chunk {
        Chunk {
            id: id.to_string(),
            content: content.to_string(),
            content_hash: crate::state::hash_content(content),
            rel_path: rel_path.to_string(),
            start_line,
            end_line,
            symbol_name: symbol_name.map(ToOwned::to_owned),
            symbol_type: None,
            language,
        }
    }

    fn search_count(index: &TextIndex, query: &str) -> usize {
        let query_parser = QueryParser::for_index(
            index.index(),
            vec![index.fields.content, index.fields.symbol_name],
        );
        let query = query_parser.parse_query(query).expect("parse query");
        index
            .reader()
            .searcher()
            .search(&query, &Count)
            .expect("count hits")
    }

    fn search_docs(index: &TextIndex, query: &str) -> Vec<TantivyDocument> {
        let query_parser = QueryParser::for_index(
            index.index(),
            vec![index.fields.content, index.fields.symbol_name],
        );
        let query = query_parser.parse_query(query).expect("parse query");
        let searcher = index.reader().searcher();
        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(10).order_by_score())
            .expect("search top docs");

        top_docs
            .into_iter()
            .map(|(_, doc_address)| searcher.doc(doc_address).expect("load stored doc"))
            .collect()
    }

    fn text_field(doc: &TantivyDocument, field: Field) -> &str {
        doc.get_first(field)
            .and_then(|value| value.as_str())
            .expect("stored text field")
    }

    fn u64_field(doc: &TantivyDocument, field: Field) -> u64 {
        doc.get_first(field)
            .and_then(|value| value.as_u64())
            .expect("stored u64 field")
    }

    fn hit_ids(hits: &[KeywordHit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.chunk_id.as_str()).collect()
    }

    fn vector_row_from_chunk(chunk: &Chunk, vector: Vec<f32>) -> ChunkRow {
        ChunkRow {
            id: chunk.id.clone(),
            content_hash: chunk.content_hash.clone(),
            vector,
            rel_path: chunk.rel_path.clone(),
            start_line: chunk.start_line.try_into().expect("start line fits u32"),
            end_line: chunk.end_line.try_into().expect("end line fits u32"),
            symbol_name: chunk.symbol_name.clone(),
            symbol_type: chunk.symbol_type.clone(),
            language: chunk
                .language
                .map(Language::as_str)
                .unwrap_or("unknown")
                .to_string(),
            content: chunk.content.clone(),
            last_modified: 1_700_000_000,
        }
    }

    #[test]
    fn new_creates_index() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());

        let first = TextIndex::new(project.path(), &config).expect("create text index");
        let tantivy_dir = first.tantivy_dir().to_path_buf();
        assert!(tantivy_dir.is_dir());
        assert_eq!(
            tantivy_dir,
            project_data_dir(project.path(), &config)
                .expect("project data dir")
                .join(TANTIVY_SUBDIR)
        );
        assert!(tantivy_dir.join("meta.json").is_file());
        assert!(first.index().tokenizers().get(EN_STEM_TOKENIZER).is_some());

        let sentinel = tantivy_dir.join("sentinel");
        fs::write(&sentinel, "keep").expect("write sentinel");
        drop(first);

        let second = TextIndex::new(project.path(), &config).expect("reopen text index");
        assert_eq!(second.tantivy_dir(), tantivy_dir.as_path());
        assert!(sentinel.is_file(), "reopen must not wipe existing dir");
    }

    #[test]
    fn schema_has_expected_fields() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());
        let index = TextIndex::new(project.path(), &config).expect("create text index");

        let schema = index.schema();
        assert_eq!(schema.num_fields(), 8);
        let names: Vec<_> = schema.fields().map(|(_, entry)| entry.name()).collect();
        assert_eq!(
            names,
            [
                "chunk_id",
                "rel_path",
                "content",
                "symbol_name",
                "language",
                "start_line",
                "end_line",
                "index_depth"
            ]
        );

        let fields = index.fields();
        assert_eq!(
            fields.chunk_id,
            schema.get_field("chunk_id").expect("chunk_id")
        );
        assert_eq!(
            fields.rel_path,
            schema.get_field("rel_path").expect("rel_path")
        );
        assert_eq!(
            fields.content,
            schema.get_field("content").expect("content")
        );
        assert_eq!(
            fields.symbol_name,
            schema.get_field("symbol_name").expect("symbol_name")
        );
        assert_eq!(
            fields.language,
            schema.get_field("language").expect("language")
        );
        assert_eq!(
            fields.start_line,
            schema.get_field("start_line").expect("start_line")
        );
        assert_eq!(
            fields.end_line,
            schema.get_field("end_line").expect("end_line")
        );
        assert_eq!(
            fields.index_depth,
            schema.get_field("index_depth").expect("index_depth")
        );

        assert_stored_only_text(schema, fields.chunk_id);
        assert_string_field(schema, fields.rel_path);
        assert_stemmed_text_field(schema, fields.content, false);
        assert_stemmed_text_field(schema, fields.symbol_name, true);
        assert_string_field(schema, fields.language);
        assert_stored_only_u64(schema, fields.start_line);
        assert_stored_only_u64(schema, fields.end_line);
        assert_string_field(schema, fields.index_depth);
    }

    #[test]
    fn add_chunks_then_commit_makes_searchable() {
        let mut fixture = text_index_fixture();
        let chunks = [chunk(
            "chunk-a",
            "src/lib.rs",
            "pub fn needle() { println!(\"found\"); }",
            Some("needle"),
            Some(Language::Rust),
            3,
            4,
        )];

        fixture.index.add_chunks(&chunks).expect("add chunks");
        assert_eq!(search_count(&fixture.index, "needle"), 0);

        fixture.index.commit().expect("commit");
        assert_eq!(search_count(&fixture.index, "needle"), 1);

        let docs = search_docs(&fixture.index, "needle");
        assert_eq!(docs.len(), 1);
        let fields = fixture.index.fields();
        assert_eq!(text_field(&docs[0], fields.chunk_id), "chunk-a");
        assert_eq!(text_field(&docs[0], fields.rel_path), "src/lib.rs");
        assert_eq!(text_field(&docs[0], fields.symbol_name), "needle");
        assert_eq!(text_field(&docs[0], fields.language), "rust");
        assert_eq!(u64_field(&docs[0], fields.start_line), 3);
        assert_eq!(u64_field(&docs[0], fields.end_line), 4);
        assert_eq!(
            text_field(&docs[0], fields.index_depth),
            PHASE_4_INDEX_DEPTH
        );
    }

    #[test]
    fn commit_is_batched_not_per_doc() {
        let mut fixture = text_index_fixture();
        let first = chunk(
            "chunk-a",
            "src/a.rs",
            "batchtoken appears in the first chunk",
            Some("first_batchtoken"),
            Some(Language::Rust),
            1,
            3,
        );
        let second = chunk(
            "chunk-b",
            "src/b.rs",
            "batchtoken appears in the second chunk",
            Some("second_batchtoken"),
            Some(Language::Rust),
            4,
            6,
        );

        fixture
            .index
            .add_chunks(std::slice::from_ref(&first))
            .expect("add first chunk");
        assert_eq!(search_count(&fixture.index, "batchtoken"), 0);

        fixture
            .index
            .add_chunks(std::slice::from_ref(&second))
            .expect("add second chunk");
        assert_eq!(search_count(&fixture.index, "batchtoken"), 0);

        fixture.index.commit().expect("commit");
        assert_eq!(search_count(&fixture.index, "batchtoken"), 2);
    }

    #[test]
    fn delete_by_file_is_scoped() {
        let mut fixture = text_index_fixture();
        let chunks = [
            chunk(
                "chunk-target",
                "src/a.rs",
                "scopedtoken alpha target file",
                Some("target_symbol"),
                Some(Language::Rust),
                1,
                2,
            ),
            chunk(
                "chunk-prefix-neighbor",
                "src/a.rs.bak",
                "scopedtoken beta prefix neighbor",
                Some("neighbor_symbol"),
                Some(Language::Rust),
                1,
                2,
            ),
            chunk(
                "chunk-other",
                "src/b.rs",
                "scopedtoken gamma other file",
                Some("other_symbol"),
                Some(Language::Rust),
                1,
                2,
            ),
        ];

        fixture.index.add_chunks(&chunks).expect("add chunks");
        fixture.index.commit().expect("initial commit");
        assert_eq!(search_count(&fixture.index, "scopedtoken"), 3);

        fixture.index.delete_by_file("src/a.rs").expect("delete");
        assert_eq!(search_count(&fixture.index, "scopedtoken"), 3);

        fixture.index.commit().expect("delete commit");
        assert_eq!(search_count(&fixture.index, "scopedtoken"), 2);
        assert_eq!(search_count(&fixture.index, "alpha"), 0);
        assert_eq!(search_count(&fixture.index, "beta"), 1);
        assert_eq!(search_count(&fixture.index, "gamma"), 1);
    }

    #[test]
    fn reindex_same_file_does_not_duplicate_chunk_ids() {
        let mut fixture = text_index_fixture();
        let original = chunk(
            "stable-chunk-id",
            "src/lib.rs",
            "duptoken original body",
            Some("duptoken_symbol"),
            Some(Language::Rust),
            1,
            3,
        );
        let replacement = chunk(
            "stable-chunk-id",
            "src/lib.rs",
            "duptoken replacement body",
            Some("duptoken_symbol"),
            Some(Language::Rust),
            1,
            4,
        );

        fixture
            .index
            .add_chunks(std::slice::from_ref(&original))
            .expect("add original");
        fixture.index.commit().expect("original commit");
        assert_eq!(search_count(&fixture.index, "duptoken"), 1);

        fixture.index.delete_by_file("src/lib.rs").expect("delete");
        fixture
            .index
            .add_chunks(std::slice::from_ref(&replacement))
            .expect("add replacement");
        fixture.index.commit().expect("replacement commit");

        let docs = search_docs(&fixture.index, "duptoken");
        assert_eq!(docs.len(), 1);
        assert_eq!(
            text_field(&docs[0], fixture.index.fields().chunk_id),
            "stable-chunk-id"
        );
        assert_eq!(u64_field(&docs[0], fixture.index.fields().end_line), 4);
    }

    #[test]
    fn search_boosts_symbol_name() {
        let mut fixture = text_index_fixture();
        let chunks = [
            chunk(
                "content-hit",
                "src/content.rs",
                "boostneedle",
                Some("content_symbol"),
                Some(Language::Rust),
                1,
                1,
            ),
            chunk(
                "symbol-hit",
                "src/symbol.rs",
                "unrelated",
                Some("boostneedle"),
                Some(Language::Rust),
                10,
                12,
            ),
        ];

        fixture.index.add_chunks(&chunks).expect("add chunks");
        fixture.index.commit().expect("commit");

        let hits = fixture.index.search("boostneedle", 2).expect("search");

        assert_eq!(hit_ids(&hits), ["symbol-hit", "content-hit"]);
        assert!(hits[0].score > hits[1].score);
        assert_eq!(hits[0].rel_path, "src/symbol.rs");
        assert_eq!(hits[0].start_line, 10);
        assert_eq!(hits[0].end_line, 12);
        assert_eq!(hits[0].symbol_name.as_deref(), Some("boostneedle"));
        assert_eq!(hits[0].language, "rust");
    }

    #[test]
    fn search_respects_top_k_and_empty() {
        let mut fixture = text_index_fixture();
        let chunks = [
            chunk(
                "top-a",
                "src/a.rs",
                "capneedle alpha",
                Some("cap_a"),
                Some(Language::Rust),
                1,
                2,
            ),
            chunk(
                "top-b",
                "src/b.rs",
                "capneedle beta",
                Some("cap_b"),
                Some(Language::Rust),
                3,
                4,
            ),
            chunk("top-c", "src/c.rs", "capneedle gamma", None, None, 5, 6),
        ];

        fixture.index.add_chunks(&chunks).expect("add chunks");
        fixture.index.commit().expect("commit");

        let hits = fixture.index.search("capneedle", 2).expect("search");
        assert_eq!(hits.len(), 2);
        assert_eq!(fixture.index.search("capneedle", 0).expect("top_k=0"), []);
        assert_eq!(fixture.index.search("", 10).expect("empty query"), []);
        assert_eq!(
            fixture
                .index
                .search(" \t\n ", 10)
                .expect("whitespace query"),
            []
        );
    }

    #[test]
    fn search_is_deterministic() {
        let mut fixture = text_index_fixture();
        let chunks = [
            chunk(
                "det-a",
                "src/a.rs",
                "detneedle detneedle detneedle",
                Some("alpha"),
                Some(Language::Rust),
                1,
                2,
            ),
            chunk(
                "det-b",
                "src/b.rs",
                "detneedle detneedle",
                Some("beta"),
                Some(Language::Rust),
                3,
                4,
            ),
            chunk(
                "det-c",
                "src/c.rs",
                "detneedle",
                Some("gamma"),
                Some(Language::Rust),
                5,
                6,
            ),
        ];

        fixture.index.add_chunks(&chunks).expect("add chunks");
        fixture.index.commit().expect("commit");

        let first = fixture.index.search("detneedle", 5).expect("search");
        assert_eq!(first.len(), 3);

        for _ in 0..5 {
            assert_eq!(fixture.index.search("detneedle", 5).expect("search"), first);
        }
    }

    #[tokio::test]
    async fn search_chunk_id_matches_vector_store_id() {
        let project = tempfile::tempdir().expect("project");
        let data_dir = tempfile::tempdir().expect("data dir");
        let config = config_with_data_dir(data_dir.path());
        let mut text_index = TextIndex::new(project.path(), &config).expect("create text index");
        let mut vector_store = VectorStore::new(project.path(), &config, SEARCH_DIM, SEARCH_MODEL)
            .await
            .expect("create vector store");
        let chunk = chunk(
            "shared-chunk-id",
            "src/shared.rs",
            "joinneedle content",
            Some("join_symbol"),
            Some(Language::Rust),
            7,
            9,
        );

        text_index
            .add_chunks(std::slice::from_ref(&chunk))
            .expect("add text chunk");
        text_index.commit().expect("commit text chunk");
        vector_store
            .insert_chunks(&[vector_row_from_chunk(&chunk, vec![1.0, 0.0, 0.0, 0.0])])
            .await
            .expect("insert vector chunk");

        let keyword_hits = text_index.search("joinneedle", 1).expect("keyword search");
        let vector_hits = vector_store
            .search(&[1.0, 0.0, 0.0, 0.0], 1, None)
            .await
            .expect("vector search");

        assert_eq!(keyword_hits.len(), 1);
        assert_eq!(vector_hits.len(), 1);
        assert_eq!(keyword_hits[0].chunk_id, vector_hits[0].id);
    }

    fn assert_stored_only_text(schema: &Schema, field: Field) {
        let entry = schema.get_field_entry(field);
        let options = text_options(schema, field);

        assert!(entry.is_stored());
        assert!(!entry.is_indexed());
        assert!(options.get_indexing_options().is_none());
    }

    fn assert_string_field(schema: &Schema, field: Field) {
        let entry = schema.get_field_entry(field);
        let indexing = text_indexing(schema, field);

        assert!(entry.is_stored());
        assert!(entry.is_indexed());
        assert_eq!(indexing.tokenizer(), "raw");
        assert_eq!(indexing.index_option(), IndexRecordOption::Basic);
    }

    fn assert_stemmed_text_field(schema: &Schema, field: Field, stored: bool) {
        let entry = schema.get_field_entry(field);
        let indexing = text_indexing(schema, field);

        assert_eq!(entry.is_stored(), stored);
        assert!(entry.is_indexed());
        assert_eq!(indexing.tokenizer(), EN_STEM_TOKENIZER);
        assert_eq!(
            indexing.index_option(),
            IndexRecordOption::WithFreqsAndPositions
        );
        assert!(indexing.index_option().has_freq());
        assert!(indexing.index_option().has_positions());
        assert!(indexing.fieldnorms());
    }

    fn assert_stored_only_u64(schema: &Schema, field: Field) {
        let entry = schema.get_field_entry(field);
        let options = numeric_options(schema, field);

        assert!(entry.is_stored());
        assert!(!entry.is_indexed());
        assert!(options.is_stored());
        assert!(!options.is_indexed());
    }

    fn text_options(schema: &Schema, field: Field) -> &TextOptions {
        match schema.get_field_entry(field).field_type() {
            FieldType::Str(options) => options,
            other => panic!("expected text field, got {other:?}"),
        }
    }

    fn text_indexing(schema: &Schema, field: Field) -> &TextFieldIndexing {
        text_options(schema, field)
            .get_indexing_options()
            .expect("text field indexing options")
    }

    fn numeric_options(schema: &Schema, field: Field) -> &NumericOptions {
        match schema.get_field_entry(field).field_type() {
            FieldType::U64(options) => options,
            other => panic!("expected u64 field, got {other:?}"),
        }
    }
}
