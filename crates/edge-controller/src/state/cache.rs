//! Non-authoritative latest-sample cache for future read APIs.
#![allow(missing_docs)]
use rhizo_storage::repo::query::LatestSample;
use rhizo_storage::{EdgeDb, StorageError};
type CacheMap = HashMap<(String, String, String), LatestSample>;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
/// Convenience cache only. Control and safety code MUST query SQLite.
#[derive(Clone, Default)]
pub struct LatestSampleCache(Arc<RwLock<CacheMap>>);
impl LatestSampleCache {
    pub async fn restore(db: &EdgeDb) -> Result<Self, StorageError> {
        let c = Self::default();
        for s in rhizo_storage::repo::query::latest_samples(db).await? {
            c.update(s)
        }
        Ok(c)
    }
    pub fn update(&self, s: LatestSample) {
        let mut g = self.0.write().unwrap_or_else(|p| p.into_inner());
        g.insert((s.device_id.clone(), s.point.clone(), s.kind.clone()), s);
    }
    pub fn len(&self) -> usize {
        self.0.read().unwrap_or_else(|p| p.into_inner()).len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
#[cfg(test)]
#[allow(
    clippy::module_inception,
    reason = "keeps the issue's literal cache:: verification filter"
)]
mod cache {
    use super::*;
    #[tokio::test]
    async fn rebuilds_from_committed_sqlite() {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        assert!(LatestSampleCache::restore(&db).await.unwrap().is_empty());
    }
}
