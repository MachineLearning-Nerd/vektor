use std::time::{SystemTime, UNIX_EPOCH};

const SECS_24H: i64 = 86_400;
const SECS_7D: i64 = 604_800;
const MIN_BOOST_SCORE: f32 = 0.3;

#[allow(dead_code)]
pub(crate) struct RecencyTracker {
    now: i64,
}

#[allow(dead_code)]
impl RecencyTracker {
    pub(crate) fn now() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_secs()).ok())
            .unwrap_or(0);
        Self { now }
    }

    pub(crate) fn at(now: i64) -> Self {
        Self { now }
    }

    pub(crate) fn score(&self, base_score: f32, mtime: i64) -> f32 {
        if base_score <= MIN_BOOST_SCORE || mtime <= 0 {
            return base_score;
        }

        let age = self.now - mtime;
        if age < 0 {
            return base_score;
        }

        base_score * recency_multiplier(age)
    }
}

fn recency_multiplier(age_secs: i64) -> f32 {
    if age_secs < SECS_24H {
        1.1
    } else if age_secs < SECS_7D {
        1.03
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    #[test]
    fn boost_within_24h() {
        let tracker = RecencyTracker::at(NOW);

        assert!((tracker.score(0.8, NOW - 60) - 0.88).abs() < 0.0001);
    }

    #[test]
    fn boost_within_7d() {
        let tracker = RecencyTracker::at(NOW);

        assert!((tracker.score(0.8, NOW - 86_400 - 1) - 0.824).abs() < 0.0001);
    }

    #[test]
    fn older_and_future_unchanged() {
        let tracker = RecencyTracker::at(NOW);

        assert_eq!(tracker.score(0.8, NOW - 604_800 - 1), 0.8);
        assert_eq!(tracker.score(0.8, NOW + 60), 0.8);
        assert_eq!(tracker.score(0.8, 0), 0.8);
    }

    #[test]
    fn min_score_gate_blocks_irrelevant() {
        let tracker = RecencyTracker::at(NOW);

        assert_eq!(tracker.score(0.2, NOW - 60), 0.2);
    }

    #[test]
    fn gate_boundary_at_0_3() {
        let tracker = RecencyTracker::at(NOW);

        assert_eq!(tracker.score(0.3, NOW - 60), 0.3);
        assert!(tracker.score(0.3001, NOW - 60) > 0.3001);
    }

    #[test]
    fn score_is_deterministic_for_fixed_now() {
        let tracker = RecencyTracker::at(NOW);

        assert_eq!(tracker.score(0.7, NOW - 100), tracker.score(0.7, NOW - 100));
    }
}
