//! Hourly bounded retention worker, and the periodic `storage_bytes` sample.
use crate::metrics::Metrics;
use tokio::sync::watch;

/// How often the `storage_bytes` gauge is refreshed.
///
/// Sampled on its own cadence rather than per request (M3-011) and far more
/// often than retention runs, because it is the signal an operator watches to
/// catch a filling disk before `SQLITE_FULL` turns every write fatal.
const SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Runs one batch per hour, samples storage size every minute, and exits on shutdown.
pub async fn run(
    db: rhizo_storage::EdgeDb,
    clock: std::sync::Arc<dyn rhizo_domain::Clock>,
    mut shutdown: watch::Receiver<bool>,
    metrics: Metrics,
) -> Result<(), String> {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
    let mut sample = tokio::time::interval(SAMPLE_INTERVAL);
    loop {
        tokio::select! {changed=shutdown.changed()=>{if changed.is_err()||*shutdown.borrow(){return Ok(())}},_=sample.tick()=>{sample_storage_bytes(&db,&metrics).await?},_=tick.tick()=>{let now=clock.now().timestamp_millis();let p=rhizo_storage::repo::retention::run_batch(&db,now,500).await.map_err(|e|e.to_string())?;for(name,n)in[("processed_messages",p.processed),("pending_cloud_events",p.outbox),("quarantined_messages",p.quarantine),("measurements",p.measurements)]{metrics.rows_pruned.with_label_values(&[name]).inc_by(n)}}}
    }
}
/// Refreshes the `storage_bytes` gauge.
///
/// A gauge is not worth the process, so only a failure that already means the
/// edge cannot continue is allowed to stop this supervised task; anything else
/// is logged and the next tick tries again.
async fn sample_storage_bytes(db: &rhizo_storage::EdgeDb, metrics: &Metrics) -> Result<(), String> {
    match rhizo_storage::repo::query::storage_bytes(db).await {
        Ok(bytes) => {
            metrics.storage_bytes.set(bytes);
            Ok(())
        }
        Err(error) if rhizo_telemetry::Classify::classify(&error).is_fatal() => {
            Err(error.to_string())
        }
        Err(error) => {
            tracing::warn!(%error, "could not sample storage_bytes");
            Ok(())
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::module_inception,
    reason = "keeps the issue's literal retention:: verification filter"
)]
mod retention {
    #[test]
    fn ledger_tables_are_not_in_retention_source() {
        let source = include_str!("../../storage/src/repo/retention.rs");
        for table in ["watering_events", "commands", "device_events"] {
            assert!(!source.contains(&format!("DELETE FROM {table}")));
        }
    }
}
