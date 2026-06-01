use std::collections::HashMap;

use crate::search::weights::AdaptiveWeights;

/// Standard Reciprocal Rank Fusion constant from PRD section 4.2.
pub const RRF_K: u32 = 60;

/// A ranked search result identifier.
///
/// RRF consumes only rank positions. Raw semantic/BM25 scores are intentionally
/// omitted because they live on incompatible scales.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedId {
    pub chunk_id: String,
}

impl RankedId {
    pub fn new(chunk_id: impl Into<String>) -> Self {
        Self {
            chunk_id: chunk_id.into(),
        }
    }
}

/// One fused search hit after weighted RRF scoring.
#[derive(Debug, Clone, PartialEq)]
pub struct FusedHit {
    pub chunk_id: String,
    pub score: f32,
}

/// Fuse semantic and keyword ranked lists with weighted Reciprocal Rank Fusion.
pub fn rrf_fuse(
    semantic: &[RankedId],
    keyword: &[RankedId],
    k: u32,
    weights: AdaptiveWeights,
) -> Vec<FusedHit> {
    let mut scores = HashMap::new();

    add_rrf_scores(&mut scores, semantic, k, weights.semantic);
    add_rrf_scores(&mut scores, keyword, k, weights.keyword);

    let mut hits = scores
        .into_iter()
        .map(|(chunk_id, score)| FusedHit { chunk_id, score })
        .collect::<Vec<_>>();

    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });

    hits
}

fn add_rrf_scores(scores: &mut HashMap<String, f32>, ranked: &[RankedId], k: u32, weight: f32) {
    for (rank, hit) in ranked.iter().enumerate() {
        let one_based_rank = rank as u32 + 1;
        let contribution = weight / (k + one_based_rank) as f32;
        *scores.entry(hit.chunk_id.clone()).or_insert(0.0) += contribution;
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn id(chunk_id: &str) -> RankedId {
        RankedId::new(chunk_id)
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < f32::EPSILON,
            "actual={actual}, expected={expected}"
        );
    }

    #[test]
    fn rrf_scores_use_one_based_rank_and_weights() {
        let weights = AdaptiveWeights {
            semantic: 0.4,
            keyword: 0.6,
        };

        let hits = rrf_fuse(&[id("same")], &[id("same")], RRF_K, weights);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk_id, "same");
        assert_close(hits[0].score, 0.4 / 61.0 + 0.6 / 61.0);
    }

    #[test]
    fn documents_from_only_one_list_are_retained() {
        let weights = AdaptiveWeights {
            semantic: 0.6,
            keyword: 0.4,
        };

        let hits = rrf_fuse(
            &[id("semantic-only")],
            &[id("keyword-only")],
            RRF_K,
            weights,
        );
        let ids = hits
            .iter()
            .map(|hit| hit.chunk_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, ["semantic-only", "keyword-only"]);
        assert_close(hits[0].score, 0.6 / 61.0);
        assert_close(hits[1].score, 0.4 / 61.0);
    }

    #[test]
    fn scores_accumulate_across_ranked_lists() {
        let weights = AdaptiveWeights {
            semantic: 0.5,
            keyword: 0.5,
        };

        let hits = rrf_fuse(
            &[id("alpha"), id("shared")],
            &[id("shared"), id("beta")],
            RRF_K,
            weights,
        );

        assert_eq!(hits[0].chunk_id, "shared");
        assert_close(hits[0].score, 0.5 / 62.0 + 0.5 / 61.0);
    }

    #[test]
    fn ties_are_sorted_by_chunk_id() {
        let weights = AdaptiveWeights {
            semantic: 0.5,
            keyword: 0.5,
        };

        let hits = rrf_fuse(&[id("b"), id("a")], &[id("a"), id("b")], RRF_K, weights);
        let ids = hits
            .iter()
            .map(|hit| hit.chunk_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, ["a", "b"]);
        assert_close(hits[0].score, hits[1].score);
    }
}
