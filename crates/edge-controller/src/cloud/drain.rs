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
        let rows = rhizo_storage::repo::outbox::ready(&db, now, size.get())
            .await
            .map_err(|e| e.to_string())?;
        let (pending, _) = rhizo_storage::repo::outbox::counts(&db)
            .await
            .map_err(|e| e.to_string())?;
        metrics.pending_cloud_events.set(pending);
        metrics.cloud_batch_size.set(i64::from(size.get()));
        // Enforce the cap independently of readiness/backoff. A prolonged
        // outage can leave every row delayed, but must not disable pruning.
        let dropped = rhizo_storage::repo::outbox::prune_low(&db)
            .await
            .map_err(|e| e.to_string())?;
        if dropped > 0 {
            metrics.cloud_events_dropped.inc_by(dropped);
            tracing::error!(dropped, "ALERT: cloud outbox cap pruned low-tier history")
        }
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
    #[tokio::test]
    async fn safety_009_decisions_identical_with_cloud_down() {
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
            "device.event",
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
