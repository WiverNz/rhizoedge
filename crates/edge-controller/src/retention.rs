//! Hourly bounded retention worker.
use crate::metrics::Metrics;
use tokio::sync::watch;
/// Runs one batch per hour and exits on shutdown.
pub async fn run(
    db: rhizo_storage::EdgeDb,
    clock: std::sync::Arc<dyn rhizo_domain::Clock>,
    mut shutdown: watch::Receiver<bool>,
    metrics: Metrics,
) -> Result<(), String> {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
    loop {
        tokio::select! {changed=shutdown.changed()=>{if changed.is_err()||*shutdown.borrow(){return Ok(())}},_=tick.tick()=>{let now=clock.now().timestamp_millis();let p=rhizo_storage::repo::retention::run_batch(&db,now,500).await.map_err(|e|e.to_string())?;for(name,n)in[("processed_messages",p.processed),("pending_cloud_events",p.outbox),("quarantined_messages",p.quarantine),("measurements",p.measurements)]{metrics.rows_pruned.with_label_values(&[name]).inc_by(n)}}}
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
