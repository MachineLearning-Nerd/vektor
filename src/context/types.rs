#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AssemblyConfig {
    pub(crate) token_budget: usize,
    pub(crate) max_files: usize,
    pub(crate) include_related: bool,
    pub(crate) min_relevance: f32,
    pub(crate) deduplicate: bool,
    pub(crate) include_docs: bool,
    pub(crate) scope: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ContextPackage {
    pub(crate) chunks: Vec<ContextChunk>,
    pub(crate) files_included: Vec<String>,
    pub(crate) total_tokens: usize,
    pub(crate) budget_used_pct: f64,
    pub(crate) missing_context_warnings: Vec<String>,
    pub(crate) search_metadata: SearchMetadata,
    pub(crate) result_confidence: Confidence,
    pub(crate) budget_gap_reason: Option<GapReason>,
    pub(crate) suggested_action: Option<String>,
    pub(crate) clusters: Vec<ResultCluster>,
    pub(crate) chunks_deduplicated: usize,
    pub(crate) index_status: String,
    pub(crate) index_coverage_pct: f64,
    pub(crate) cache_hit: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SearchMetadata {
    pub(crate) search_time_ms: u128,
    pub(crate) direct_results: usize,
    pub(crate) expanded_results: usize,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Confidence {
    High,
    Medium,
    Low,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GapReason {
    NoMoreRelevant,
    IndexIncomplete,
    ThresholdFiltered,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResultCluster {
    pub(crate) path_prefix: String,
    pub(crate) chunk_count: usize,
    pub(crate) avg_relevance: f32,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChunkSource {
    Search,
    Related,
    Dependency,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ContextChunk {
    pub(crate) chunk_id: String,
    pub(crate) content: String,
    pub(crate) rel_path: String,
    pub(crate) lines: (u64, u64),
    pub(crate) symbol: Option<String>,
    pub(crate) symbol_type: Option<String>,
    pub(crate) language: String,
    pub(crate) relevance_score: f32,
    pub(crate) source: ChunkSource,
    pub(crate) reason: String,
    pub(crate) is_expanded: bool,
    pub(crate) last_modified: i64,
}
