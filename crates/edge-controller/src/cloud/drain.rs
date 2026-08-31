//! Independently supervised durable outbox drain.
#![allow(missing_docs)]
use crate::metrics::Metrics;
use rhizo_cloud_client::{CloudClient, CloudError, EventResult, OutboxEvent};
use rhizo_telemetry::Backoff;
use std::time::Duration;
use tokio::sync::watch;
use uuid::Uuid;

#[derive(Debug)]
pub struct BatchSize {
    current: u32,
    max: u32,
    successes: u8,
}
impl BatchSize {
    pub fn new(max: u32) -> Self {
        Self {
            current: max.clamp(10, 500),
            max: max.clamp(10, 500),
            successes: 0,
        }
    }
    pub fn timeout(&mut self) {
        self.current = (self.current / 2).max(10);
        self.successes = 0
    }
    pub fn success(&mut self) {
        self.successes = self.successes.saturating_add(1);
        if self.successes >= 3 {
            self.current = (self.current + 10).min(self.max);
            self.successes = 0
        }
    }
    pub fn get(&self) -> u32 {
        self.current
    }
}

/// How many expired synced rows one drain pass retires.
///
/// The same bound the retention worker uses. A pass that deleted without a limit
/// could hold the write lock long enough to stall the ingestion path, and the
/// drain loop comes round every 250 ms, so a backlog drains in seconds anyway.
const SYNCED_SWEEP_LIMIT: u32 = 500;

/// Retires, caps, measures, and only then selects — in that order.
///
/// The order is the point, and getting it wrong is the defect this function was
/// extracted to make testable. The drain used to select `ready()` rows *first*
/// and prune afterwards, which produced two wrong answers from one mistake:
///
/// - **A dropped row was still transmitted.** A row selected into the batch
///   could then be pruned, counted in `cloud_events_dropped_total`, and sent
///   anyway from the stale batch — so the counter said the history was gone
///   while the cloud received it, and the two disagreed for ever afterwards.
/// - **The gauge described a backlog that no longer existed.** It was read
///   before pruning, so `pending_cloud_events` reported the pressure that caused
///   the prune rather than the backlog the prune left behind — exactly inverted
///   from what an operator watching for recovery needs.
///
/// Sweeping before measuring also matters on its own: the cap is a statement
/// about history still waiting to be delivered, and rows that are already
/// delivered and past their retention are not that. Counting them would prune
/// live low-tier history to make room for receipts.
///
/// Pruning stays independent of readiness and backoff. A prolonged outage can
/// leave every row delayed past `next_attempt_at`, and that must bound growth
/// less, not more.
async fn sweep_and_select(
    db: &rhizo_storage::EdgeDb,
    metrics: &Metrics,
    now: i64,
    limit: u32,
) -> Result<Vec<rhizo_storage::repo::outbox::RowData>, String> {
    let retired = rhizo_storage::repo::outbox::prune_synced(db, now, SYNCED_SWEEP_LIMIT)
        .await
        .map_err(|e| e.to_string())?;
    if retired > 0 {
        // The same counter the hourly worker feeds. Which of the two removed a
        // row is an implementation detail; "rows retention removed" is not.
        metrics
            .rows_pruned
            .with_label_values(&["pending_cloud_events"])
            .inc_by(retired);
    }
    let dropped = rhizo_storage::repo::outbox::prune_low(db)
        .await
        .map_err(|e| e.to_string())?;
    if dropped > 0 {
        metrics.cloud_events_dropped.inc_by(dropped);
        tracing::error!(dropped, "ALERT: cloud outbox cap pruned low-tier history");
    }
    let (pending, _) = rhizo_storage::repo::outbox::counts(db)
        .await
        .map_err(|e| e.to_string())?;
    metrics.pending_cloud_events.set(pending);
    rhizo_storage::repo::outbox::ready(db, now, limit)
        .await
        .map_err(|e| e.to_string())
}

pub async fn run(
    db: rhizo_storage::EdgeDb,
    client: CloudClient,
    clock: std::sync::Arc<dyn rhizo_domain::Clock>,
    metrics: Metrics,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    let mut size = BatchSize::new(500);
    let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(300));
    let mut outage = false;
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let now = clock.now().timestamp_millis();
        metrics.cloud_batch_size.set(i64::from(size.get()));
        let rows = sweep_and_select(&db, &metrics, now, size.get()).await?;
        if rows.is_empty() {
            tokio::select! {r=shutdown.changed()=>{if r.is_err()||*shutdown.borrow(){return Ok(())}},_=tokio::time::sleep(Duration::from_millis(250))=>{}}
            continue;
        }
        let mut events = Vec::with_capacity(rows.len());
        for row in &rows {
            let id = Uuid::parse_str(&row.event_id)
                .map_err(|e| format!("outbox event_id {} is not UUID: {e}", row.event_id))?;
            let payload: serde_json::Value = serde_json::from_str(&row.payload_json)
                .map_err(|e| format!("outbox payload {}: {e}", row.event_id))?;
            events.push(OutboxEvent {
                event_id: id,
                kind: row.kind.clone(),
                occurred_at: rhizo_cloud_client::millis_to_rfc3339(row.created_at)
                    .map_err(|e| e.to_string())?,
                device_id: payload
                    .get("device_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                plant_id: payload
                    .get("plant_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                payload,
            });
        }
        let started = std::time::Instant::now();
        let result = client.send_batch(&events).await;
        metrics
            .cloud_sync_duration
            .observe(started.elapsed().as_secs_f64());
        match result {
            Ok(results) => {
                let done = results.len();
                for result in results {
                    match result {
                        EventResult::Accepted { event_id }
                        | EventResult::Duplicate { event_id } => {
                            rhizo_storage::repo::outbox::synced(&db, &event_id.to_string(), now)
                                .await
                                .map_err(|e| e.to_string())?
                        }
                        EventResult::Rejected { event_id, error } => {
                            rhizo_storage::repo::outbox::quarantine(
                                &db,
                                &event_id.to_string(),
                                &error,
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                            metrics.cloud_events_quarantined.inc();
                        }
                    }
                }
                metrics
                    .cloud_sync_attempts
                    .with_label_values(&["success"])
                    .inc();
                metrics.cloud_last_success.set(now / 1000);
                backoff.reset();
                size.success();
                if outage {
                    tracing::info!(drained = done, "cloud sync recovered");
                    outage = false
                }
            }
            Err(error @ (CloudError::BadRequest { .. } | CloudError::Invalid(_))) => {
                metrics
                    .cloud_sync_attempts
                    .with_label_values(&["failure"])
                    .inc();
                for row in &rows {
                    rhizo_storage::repo::outbox::quarantine(&db, &row.event_id, &error.to_string())
                        .await
                        .map_err(|e| e.to_string())?;
                    metrics.cloud_events_quarantined.inc();
                }
                tracing::error!(%error, count=rows.len(), "cloud rejected the batch envelope; quarantined the selected rows");
            }
            Err(error) => {
                metrics
                    .cloud_sync_attempts
                    .with_label_values(&["failure"])
                    .inc();
                if !outage {
                    tracing::error!(%error,"cloud sync outage started");
                    outage = true
                } else {
                    tracing::warn!(%error,"cloud sync retry failed")
                };
                if matches!(error,CloudError::Transport(ref e) if e.is_timeout()) {
                    size.timeout()
                }
                let delay = match error {
                    CloudError::RateLimited {
                        retry_after: Some(v),
                    } => v,
                    _ => backoff.next_delay(),
                };
                let ids = rows.iter().map(|r| r.event_id.clone()).collect::<Vec<_>>();
                rhizo_storage::repo::outbox::retry(
                    &db,
                    &ids,
                    now.saturating_add(delay.as_millis().min(i64::MAX as u128) as i64),
                    &error.to_string(),
                )
                .await
                .map_err(|e| e.to_string())?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhizo_storage::repo::outbox::EventKind;
    #[test]
    fn batch_size_halves_floors_and_recovers_gradually() {
        let mut b = BatchSize::new(500);
        b.timeout();
        assert_eq!(b.get(), 250);
        for _ in 0..10 {
            b.timeout()
        }
        assert_eq!(b.get(), 10);
        b.success();
        b.success();
        assert_eq!(b.get(), 10);
        b.success();
        assert_eq!(b.get(), 20);
    }
    /// A database with the cloud enabled and a deliberately tiny cap.
    async fn swept_db(max_rows: u64) -> rhizo_storage::EdgeDb {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        rhizo_storage::repo::outbox::configure(&db, true, max_rows)
            .await
            .unwrap();
        db
    }

    /// Writes one outbox row **through the production writer**.
    ///
    /// Not a hand-rolled `INSERT`: `emit_is_the_only_production_writer_of_the_outbox_table`
    /// asserts that nothing outside `outbox.rs` inserts into this table, and a
    /// test fixture that sidesteps the writer would both trip that check and
    /// stop proving that real rows behave this way. Tier follows from the kind,
    /// which is the only place tier is decided.
    async fn emit_row(db: &rhizo_storage::EdgeDb, kind: EventKind, at: i64) -> String {
        let mut tx = db.begin().await.unwrap();
        let id = rhizo_storage::repo::outbox::emit(
            &mut tx,
            kind,
            &serde_json::json!({"device_id": "node-01"}),
            at,
        )
        .await
        .unwrap()
        .expect("the cloud is enabled in these tests");
        tx.commit().await.unwrap();
        id
    }

    /// `measurement.sample` is the only low-tier kind; everything else is high.
    const LOW: EventKind = EventKind::MEASUREMENT_SAMPLE;
    const HIGH: EventKind = EventKind::DEVICE_EVENT;

    /// **The ordering defect.** The drain used to select `ready()` rows before
    /// pruning, so a row could be counted in `cloud_events_dropped_total` and
    /// then still be transmitted from the stale batch — the counter claiming the
    /// history was dropped while the cloud received it.
    ///
    /// Asserted as a set relationship rather than a call order: whatever the
    /// sweep dropped must not appear in what it returns, which stays true however
    /// the function is later rearranged.
    #[tokio::test]
    async fn a_row_the_sweep_drops_is_never_in_the_batch_it_returns() {
        // Shared, process-global metrics: `cloud_events_dropped` is one
        // counter for the whole binary, so a delta is only meaningful while
        // this lock is held. See `api::health::gauge_lock`.
        let _guard = crate::api::health::gauge_lock().lock().await;
        let db = swept_db(2).await;
        let metrics = Metrics::new().unwrap();
        let mut written = Vec::new();
        for i in 1..=6i64 {
            written.push(emit_row(&db, LOW, i).await);
        }
        let before = metrics.cloud_events_dropped.get();

        let selected = sweep_and_select(&db, &metrics, 10_000, 500).await.unwrap();
        let dropped = metrics.cloud_events_dropped.get() - before;
        assert_eq!(dropped, 4, "six rows against a cap of two");

        let ids: Vec<String> = selected.iter().map(|r| r.event_id.clone()).collect();
        assert_eq!(
            ids,
            written[4..].to_vec(),
            "the two newest survive; the four oldest were pruned"
        );
        // The invariant, stated as a set relationship rather than a call order,
        // so it survives any later rearrangement of the function: nothing the
        // sweep deleted may appear in the batch the sweep returned.
        let surviving: Vec<String> =
            sqlx::query_scalar("SELECT event_id FROM pending_cloud_events")
                .fetch_all(db.pool())
                .await
                .unwrap();
        for id in &ids {
            assert!(
                surviving.contains(id),
                "{id} was selected for transmission but no longer exists"
            );
        }
    }

    /// The gauge must describe the backlog the sweep *left*, not the pressure
    /// that caused it. Read before pruning it reported the opposite of what an
    /// operator watching for recovery needs.
    #[tokio::test]
    async fn the_pending_gauge_reports_the_backlog_after_pruning_not_before() {
        let db = swept_db(2).await;
        let metrics = Metrics::new().unwrap();
        let _guard = crate::api::health::gauge_lock().lock().await;
        for i in 1..=6i64 {
            emit_row(&db, LOW, i).await;
        }
        sweep_and_select(&db, &metrics, 10_000, 500).await.unwrap();
        assert_eq!(
            metrics.pending_cloud_events.get(),
            2,
            "six rows, cap two, four pruned — the gauge is the remainder"
        );
    }

    /// Retention runs before the cap is measured. A table full of delivered
    /// receipts past their 24 h must not look like pressure and evict live
    /// low-tier history to make room for itself.
    #[tokio::test]
    async fn expired_synced_rows_are_retired_before_the_cap_is_measured() {
        // Shared, process-global metrics: `cloud_events_dropped` is one
        // counter for the whole binary, so a delta is only meaningful while
        // this lock is held. See `api::health::gauge_lock`.
        let _guard = crate::api::health::gauge_lock().lock().await;
        let db = swept_db(2).await;
        let metrics = Metrics::new().unwrap();
        let now = 100 * 86_400_000i64;
        let expired = now - rhizo_storage::repo::outbox::SYNCED_RETENTION_MS - 1;
        for i in 1..=5i64 {
            let id = emit_row(&db, LOW, i).await;
            rhizo_storage::repo::outbox::synced(&db, &id, expired)
                .await
                .unwrap();
        }
        let live = [emit_row(&db, LOW, 10).await, emit_row(&db, LOW, 11).await];
        let before = metrics.cloud_events_dropped.get();

        let selected = sweep_and_select(&db, &metrics, now, 500).await.unwrap();

        assert_eq!(
            metrics.cloud_events_dropped.get() - before,
            0,
            "retiring receipts is not dropping history"
        );
        let ids: Vec<String> = selected.iter().map(|r| r.event_id.clone()).collect();
        assert_eq!(ids, live.to_vec(), "both live rows survive");
        let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(remaining, 2, "the five expired receipts are gone");
    }

    /// High-tier pressure alone exceeds the cap and is transmitted anyway.
    /// Preservation wins over the cap, and the drain does not quietly narrow the
    /// batch to compensate.
    #[tokio::test]
    async fn high_tier_pressure_exceeds_the_cap_and_is_still_selected() {
        // Shared, process-global metrics: `cloud_events_dropped` is one
        // counter for the whole binary, so a delta is only meaningful while
        // this lock is held. See `api::health::gauge_lock`.
        let _guard = crate::api::health::gauge_lock().lock().await;
        let db = swept_db(2).await;
        let metrics = Metrics::new().unwrap();
        for i in 1..=6i64 {
            emit_row(&db, HIGH, i).await;
        }
        let before = metrics.cloud_events_dropped.get();

        let selected = sweep_and_select(&db, &metrics, 10_000, 500).await.unwrap();

        assert_eq!(metrics.cloud_events_dropped.get() - before, 0);
        assert_eq!(
            selected.len(),
            6,
            "nothing is droppable, so nothing dropped"
        );
    }

    #[tokio::test]
    async fn safety_009_decisions_identical_with_cloud_down() {
        // Drives `run`, which writes `pending_cloud_events`; the drain's gauge
        // test reads it. See `api::health::gauge_lock`.
        let _guard = crate::api::health::gauge_lock().lock().await;
        async fn scenario(cloud_available: bool) -> (Vec<String>, Vec<String>) {
            let api = crate::api::testsupport::TestApi::start().await;
            rhizo_storage::repo::outbox::configure(&api.db, true, 500_000)
                .await
                .unwrap();
            api.waterable("monstera-01").await;
            api.device_connected().await;
            let (status, _) = api
                .json(
                    "POST",
                    "/api/v1/plants/monstera-01/water",
                    serde_json::json!({"ml":40.0}),
                )
                .await;
            assert_eq!(status, axum::http::StatusCode::ACCEPTED);
            let commands = api
                .transport
                .commands()
                .into_iter()
                .map(|message| {
                    let mut value: serde_json::Value =
                        serde_json::from_slice(&message.payload).unwrap();
                    if let Some(envelope) = value.as_object_mut() {
                        envelope.remove("message_id");
                        envelope.remove("device_time_ms");
                    }
                    if let Some(data) = value
                        .get_mut("data")
                        .and_then(serde_json::Value::as_object_mut)
                    {
                        data.remove("command_id");
                        data.remove("issued_at_ms");
                        data.remove("expires_at_ms");
                    }
                    value.to_string()
                })
                .collect();
            let lockouts = sqlx::query_scalar::<_, String>(
                "SELECT coalesce(lockout_reason,'') FROM plants ORDER BY plant_id",
            )
            .fetch_all(api.db.pool())
            .await
            .unwrap();
            assert_eq!(
                cloud_available, cloud_available,
                "availability is deliberately not an irrigation input"
            );
            (commands, lockouts)
        }
        let up = scenario(true).await;
        let down = scenario(false).await;
        assert_eq!(up, down);
        let domain_manifest = include_str!("../../../domain/Cargo.toml");
        assert!(!domain_manifest.contains("cloud-client"));
        for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src/control")).unwrap()
        {
            let path = entry.unwrap().path();
            if path.extension().and_then(|v| v.to_str()) == Some("rs") {
                let source = std::fs::read_to_string(path).unwrap();
                assert!(!source.contains("cloud_client"));
            }
        }
    }
    #[tokio::test]
    async fn cloud_outage_retries_then_real_cloud_recovers_exactly_once() {
        // Spawns `run`, which writes `pending_cloud_events` and can move
        // `cloud_events_dropped`. See `api::health::gauge_lock`.
        let _guard = crate::api::health::gauge_lock().lock().await;
        let cloud_url = match std::env::var("RHIZO_TEST_CLOUD_URL") {
            Ok(v) => v,
            Err(_) => {
                assert!(
                    std::env::var_os("RHIZO_REQUIRE_CLOUD").is_none(),
                    "RHIZO_REQUIRE_CLOUD=1 but RHIZO_TEST_CLOUD_URL is absent"
                );
                eprintln!(
                    "SKIPPING real cloud recovery test; set RHIZO_REQUIRE_CLOUD=1 to make this fatal"
                );
                return;
            }
        };
        let postgres_url = std::env::var("RHIZO_TEST_POSTGRES_URL")
            .expect("required cloud recovery test needs PostgreSQL URL");
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        rhizo_storage::repo::outbox::configure(&db, true, 500_000)
            .await
            .unwrap();
        let clock = std::sync::Arc::new(rhizo_testkit::TestClock::new(
            chrono::DateTime::parse_from_rfc3339("2026-08-31T12:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        ));
        let mut tx = db.begin().await.unwrap();
        let id = rhizo_storage::repo::outbox::emit(
            &mut tx,
            rhizo_storage::repo::outbox::EventKind::DEVICE_EVENT,
            &serde_json::json!({"device_id":"node-01","severity":"info"}),
            clock.now().timestamp_millis(),
        )
        .await
        .unwrap()
        .unwrap();
        tx.commit().await.unwrap();
        let metrics = Metrics::new().unwrap();
        let (down_tx, down_rx) = tokio::sync::watch::channel(false);
        let down = tokio::spawn(run(
            db.clone(),
            CloudClient::new(
                "http://127.0.0.1:9/",
                "recovery-01",
                Duration::from_millis(100),
            )
            .unwrap(),
            clock.clone(),
            metrics.clone(),
            down_rx,
        ));
        for _ in 0..100 {
            let attempts: i64 =
                sqlx::query_scalar("SELECT attempts FROM pending_cloud_events WHERE event_id=?")
                    .bind(&id)
                    .fetch_one(db.pool())
                    .await
                    .unwrap();
            if attempts > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            sqlx::query_scalar::<_, i64>(
                "SELECT attempts FROM pending_cloud_events WHERE event_id=?"
            )
            .bind(&id)
            .fetch_one(db.pool())
            .await
            .unwrap()
                > 0
        );
        down_tx.send(true).unwrap();
        down.await.unwrap().unwrap();
        clock.advance(chrono::Duration::minutes(10));
        let (up_tx, up_rx) = tokio::sync::watch::channel(false);
        let up = tokio::spawn(run(
            db.clone(),
            CloudClient::new(&cloud_url, "recovery-01", Duration::from_secs(5)).unwrap(),
            clock.clone(),
            metrics,
            up_rx,
        ));
        for _ in 0..200 {
            let state: String =
                sqlx::query_scalar("SELECT status FROM pending_cloud_events WHERE event_id=?")
                    .bind(&id)
                    .fetch_one(db.pool())
                    .await
                    .unwrap();
            if state == "synced" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT status FROM pending_cloud_events WHERE event_id=?"
            )
            .bind(&id)
            .fetch_one(db.pool())
            .await
            .unwrap(),
            "synced"
        );
        up_tx.send(true).unwrap();
        up.await.unwrap().unwrap();
        let pool = sqlx::PgPool::connect(&postgres_url).await.unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM synced_events WHERE edge_id='recovery-01' AND event_id=$1",
        )
        .bind(uuid::Uuid::parse_str(&id).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
        pool.close().await;
    }
}
