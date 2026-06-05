use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::{
    config::Config, state::hash_content, text_index::TextIndex, vector_store::VectorStore,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum IndexPhase {
    Building,
    Partial,
    Full,
}

impl IndexPhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Building => "building",
            Self::Partial => "partial",
            Self::Full => "full",
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct IndexStatusTracker {
    phases: Arc<RwLock<HashMap<String, IndexPhase>>>,
}

impl IndexStatusTracker {
    #[allow(dead_code)]
    pub(crate) fn new(project_keys: impl IntoIterator<Item = String>) -> Self {
        let tracker = Self::default();
        {
            let mut phases = tracker.write_phases();
            for project_key in project_keys {
                phases.insert(project_key, IndexPhase::Building);
            }
        }
        tracker
    }

    pub(crate) fn set_phase(&self, project_key: &str, phase: IndexPhase) {
        self.write_phases().insert(project_key.to_string(), phase);
    }

    pub(crate) fn mark_building(&self, project_key: &str) {
        self.set_phase(project_key, IndexPhase::Building);
    }

    pub(crate) fn mark_partial(&self, project_key: &str) {
        let mut phases = self.write_phases();
        let current = phases
            .get(project_key)
            .copied()
            .unwrap_or(IndexPhase::Building);
        if current < IndexPhase::Full {
            phases.insert(project_key.to_string(), IndexPhase::Partial);
        }
    }

    pub(crate) fn mark_full(&self, project_key: &str) {
        self.set_phase(project_key, IndexPhase::Full);
    }

    #[allow(dead_code)]
    pub(crate) fn status(&self, project_key: &str) -> IndexPhase {
        self.read_phases()
            .get(project_key)
            .copied()
            .unwrap_or(IndexPhase::Building)
    }

    pub(crate) fn status_for_root(&self, root: &Path, config: &Config) -> IndexPhase {
        let project_key = project_key(root);
        if let Some(phase) = self.read_phases().get(&project_key).copied() {
            return phase;
        }

        let phase = phase_from_disk(root, config);
        self.write_phases().insert(project_key, phase);
        phase
    }

    fn read_phases(&self) -> RwLockReadGuard<'_, HashMap<String, IndexPhase>> {
        match self.phases.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn write_phases(&self) -> RwLockWriteGuard<'_, HashMap<String, IndexPhase>> {
        match self.phases.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

pub(crate) fn project_key(root: &Path) -> String {
    hash_content(&root.to_string_lossy())
}

pub(crate) fn phase_from_disk(root: &Path, config: &Config) -> IndexPhase {
    let text_searchable = TextIndex::open_readonly(root, config)
        .map(|index| index.is_searchable())
        .unwrap_or(false);
    if !text_searchable {
        return IndexPhase::Building;
    }

    let vector_full = VectorStore::load_meta(root, config)
        .ok()
        .flatten()
        .and_then(|meta| meta.last_full_index_at)
        .is_some();
    if vector_full {
        IndexPhase::Full
    } else {
        IndexPhase::Partial
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chunker::Chunk, config::Config, shallow_indexer::ShallowIndexer, text_index::TextIndex,
        vector_store::VectorStore,
    };

    fn config_for(data_dir: &std::path::Path) -> Config {
        let mut config = Config::default();
        config.index.data_dir = data_dir.to_string_lossy().into_owned();
        config
    }

    #[test]
    fn index_phase_strings_are_stable() {
        assert_eq!(IndexPhase::Building.as_str(), "building");
        assert_eq!(IndexPhase::Partial.as_str(), "partial");
        assert_eq!(IndexPhase::Full.as_str(), "full");
    }

    #[test]
    fn tracker_is_per_project_and_forward_only_until_reset() {
        let tracker = IndexStatusTracker::new(["project-a".to_string()]);

        assert_eq!(tracker.status("project-a"), IndexPhase::Building);
        assert_eq!(tracker.status("project-b"), IndexPhase::Building);

        tracker.mark_partial("project-a");
        assert_eq!(tracker.status("project-a"), IndexPhase::Partial);
        assert_eq!(tracker.status("project-b"), IndexPhase::Building);

        tracker.mark_full("project-a");
        tracker.mark_partial("project-a");
        assert_eq!(tracker.status("project-a"), IndexPhase::Full);

        tracker.mark_building("project-a");
        assert_eq!(tracker.status("project-a"), IndexPhase::Building);
    }

    #[tokio::test]
    async fn tracker_seeds_partial_and_full_from_disk() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let data_dir = tempdir.path().join("data");
        let config = config_for(&data_dir);

        let partial_repo = tempdir.path().join("partial");
        std::fs::create_dir_all(partial_repo.join("src")).expect("mkdir partial");
        std::fs::write(
            partial_repo.join("src/lib.rs"),
            "pub fn partialseedneedle() -> bool { true }\n",
        )
        .expect("write partial");
        ShallowIndexer::build(&partial_repo, &config).expect("build shallow");

        let full_repo = tempdir.path().join("full");
        std::fs::create_dir_all(full_repo.join("src")).expect("mkdir full");
        let mut text_index = TextIndex::new(&full_repo, &config).expect("text index");
        let chunk = Chunk::new(
            "pub fn fullseedneedle() -> bool { true }\n".to_string(),
            "src/lib.rs".to_string(),
            1,
            1,
            Some("fullseedneedle".to_string()),
            Some("function".to_string()),
            None,
        );
        text_index.add_chunks(&[chunk]).expect("add chunk");
        text_index.commit().expect("commit ready");
        let mut store = VectorStore::new(&full_repo, &config, 4, "test-model")
            .await
            .expect("vector store");
        store
            .mark_full_index_completed(1_700_000_000)
            .expect("mark full");

        let tracker = IndexStatusTracker::default();

        assert_eq!(
            tracker.status_for_root(tempdir.path().join("missing").as_path(), &config),
            IndexPhase::Building
        );
        assert_eq!(
            tracker.status_for_root(&partial_repo, &config),
            IndexPhase::Partial
        );
        assert_eq!(
            tracker.status_for_root(&full_repo, &config),
            IndexPhase::Full
        );
    }
}
