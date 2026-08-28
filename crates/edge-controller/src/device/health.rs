//! Derived liveness, staleness, drift, and periodic time refresh.
use crate::metrics::Metrics;
use rhizo_domain::Clock;
use rhizo_mqtt_contract::{Envelope, MessageId, MessageKind, Topic, UtcMillis};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

/// The protocol's conservative minimum freshness window.
pub const STALE_FLOOR_SECONDS: i64 = 15 * 60;

/// Calculates the threshold without consulting device time.
pub const fn stale_after_seconds(telemetry_interval_seconds: i64) -> i64 {
    let tripled = telemetry_interval_seconds.saturating_mul(3);
    if tripled > STALE_FLOOR_SECONDS {
        tripled
    } else {
        STALE_FLOOR_SECONDS
    }
}

/// Derives age from edge receipt time.
pub fn sample_age_seconds(now_ms: i64, received_at_ms: i64) -> i64 {
    now_ms.saturating_sub(received_at_ms).max(0) / 1000
}

/// Runs even when no messages arrive, keeping liveness and time sync honest.
pub async fn run(
    db: rhizo_storage::EdgeDb,
    clock: Arc<dyn Clock>,
    client: rumqttc::AsyncClient,
    metrics: Metrics,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() { return Ok(()); },
            _ = interval.tick() => tick(&db, clock.as_ref(), &client, &metrics).await.map_err(|e| e.to_string())?,
        }
    }
}

async fn tick(
    db: &rhizo_storage::EdgeDb,
    clock: &dyn Clock,
    client: &rumqttc::AsyncClient,
    metrics: &Metrics,
) -> Result<(), rhizo_storage::StorageError> {
    use sqlx::Row as _;
    let now = clock.now().timestamp_millis();
    let rows = sqlx::query("SELECT device_id,status,last_seen_at,telemetry_interval_seconds,desired_config_version,applied_config_version,drift_since,connectivity_mode,last_time_sync_at,power_mode,wake_interval_seconds,overdue_at FROM devices")
        .fetch_all(db.pool()).await.map_err(|e| rhizo_storage::StorageError::Database(e.to_string()))?;
    let mut online = 0;
    let mut offline = 0;
    let mut isolated = 0;
    let mut sleeping = 0;
    for row in rows {
        let device: String = row.get("device_id");
        let status: String = row.get("status");
        let last_seen: Option<i64> = row.get("last_seen_at");
        let telemetry: i64 = row.get("telemetry_interval_seconds");
        let power_mode: String = row.get("power_mode");
        let wake_interval: Option<i64> = row.get("wake_interval_seconds");
        let effective_interval = if power_mode == "battery" {
            wake_interval.unwrap_or(telemetry)
        } else {
            telemetry
        };
        let stale = last_seen.is_some_and(|seen| {
            sample_age_seconds(now, seen) >= stale_after_seconds(effective_interval)
        });
        if stale {
            let id = format!("edge:{device}:stale:{}", last_seen.unwrap_or_default());
            sqlx::query("INSERT OR IGNORE INTO device_events(event_id,device_id,kind,severity,occurred_at,received_at,origin) VALUES(?,?,'stale','warning',?,?,'edge')")
                .bind(id).bind(&device).bind(now).bind(now).execute(db.pool()).await.map_err(|e| rhizo_storage::StorageError::Database(e.to_string()))?;
        }
        let mut connectivity: String = row.get("connectivity_mode");
        let overdue_at: Option<i64> = row.get("overdue_at");
        if connectivity == "sleeping" && overdue_at.is_some_and(|deadline| now >= deadline) {
            let id = format!(
                "edge:{device}:device_wake_missed:{}",
                overdue_at.unwrap_or_default()
            );
            let mut tx = db.begin().await?;
            let changed = sqlx::query("UPDATE devices SET connectivity_mode='isolated',missed_wake_count=missed_wake_count+1 WHERE device_id=? AND connectivity_mode='sleeping' AND overdue_at<=?")
                .bind(&device).bind(now).execute(&mut *tx).await.map_err(|e| rhizo_storage::StorageError::Database(e.to_string()))?;
            if changed.rows_affected() == 1 {
                sqlx::query("INSERT OR IGNORE INTO device_events(event_id,device_id,kind,severity,detail_json,occurred_at,received_at,origin) VALUES(?,?,'device_wake_missed','warning',?, ?,?,'edge')")
                    .bind(id).bind(&device)
                    .bind(serde_json::json!({"expected_wake_at": overdue_at}).to_string())
                    .bind(now).bind(now).execute(&mut *tx).await.map_err(|e| rhizo_storage::StorageError::Database(e.to_string()))?;
                tx.commit()
                    .await
                    .map_err(|e| rhizo_storage::StorageError::Database(e.to_string()))?;
                metrics.device_wake_missed.inc();
                connectivity = "isolated".to_owned();
            } else {
                tx.rollback()
                    .await
                    .map_err(|e| rhizo_storage::StorageError::Database(e.to_string()))?;
            }
        }
        if status == "online" {
            online += 1;
        } else if connectivity != "sleeping" {
            offline += 1;
        }
        if connectivity == "isolated" {
            isolated += 1;
        } else if connectivity == "sleeping" {
            sleeping += 1;
        }
        let desired: i64 = row.get("desired_config_version");
        let applied: Option<i64> = row.get("applied_config_version");
        let drift_since: Option<i64> = row.get("drift_since");
        if applied == Some(desired) {
            if drift_since.is_some() {
                sqlx::query("UPDATE devices SET drift_since=NULL WHERE device_id=?")
                    .bind(&device)
                    .execute(db.pool())
                    .await
                    .map_err(|e| rhizo_storage::StorageError::Database(e.to_string()))?;
            }
        } else if let Some(since) = drift_since {
            if now.saturating_sub(since) >= telemetry.saturating_mul(2000) {
                let id = format!("edge:{device}:config_drift:{desired}");
                sqlx::query("INSERT OR IGNORE INTO device_events(event_id,device_id,kind,severity,occurred_at,received_at,origin) VALUES(?,?,'config_drift','warning',?,?,'edge')")
                    .bind(id).bind(&device).bind(now).bind(now).execute(db.pool()).await.map_err(|e| rhizo_storage::StorageError::Database(e.to_string()))?;
            }
        } else {
            sqlx::query("UPDATE devices SET drift_since=? WHERE device_id=?")
                .bind(now)
                .bind(&device)
                .execute(db.pool())
                .await
                .map_err(|e| rhizo_storage::StorageError::Database(e.to_string()))?;
        }
        let last_sync: Option<i64> = row.get("last_time_sync_at");
        if status == "online"
            && last_sync.is_none_or(|last| now.saturating_sub(last) >= 300_000)
            && let Ok(device_id) = device.parse()
        {
            let envelope = Envelope {
                v: 1,
                kind: MessageKind::EdgeTime,
                message_id: MessageId::from_uuid(uuid::Uuid::new_v4()),
                device_id,
                boot_id: None,
                sequence: None,
                device_time_ms: None,
                clock_synced: None,
                data: rhizo_mqtt_contract::payload::EdgeTime {
                    edge_time_ms: UtcMillis(now),
                },
            };
            if let Ok(payload) = envelope.to_json() {
                let published = client
                    .publish(
                        Topic::Time(envelope.device_id.clone()).as_string(),
                        rumqttc::QoS::AtLeastOnce,
                        false,
                        payload,
                    )
                    .await;
                if published.is_ok() {
                    sqlx::query("UPDATE devices SET last_time_sync_at=? WHERE device_id=?")
                        .bind(now)
                        .bind(&device)
                        .execute(db.pool())
                        .await
                        .map_err(|e| rhizo_storage::StorageError::Database(e.to_string()))?;
                } else if let Err(error) = published {
                    tracing::warn!(%device, %error, "periodic edge.time publish deferred");
                }
            }
        }
    }
    metrics.devices_online.set(online);
    metrics.devices_offline.set(offline);
    metrics.devices_isolated.set(isolated);
    metrics.devices_sleeping.set(sleeping);
    Ok(())
}

#[cfg(test)]
mod staleness {
    use super::*;
    #[test]
    fn floor_and_interval() {
        assert_eq!(stale_after_seconds(10), 900);
        assert_eq!(stale_after_seconds(400), 1200);
    }
    #[test]
    fn edge_receipt_time_only() {
        assert_eq!(sample_age_seconds(20_000, 5_000), 15);
    }
    #[tokio::test]
    async fn safety_021_overdue_sleeper_becomes_isolated_without_inbound_message() {
        use chrono::{TimeZone, Utc};
        use rhizo_mqtt_contract::payload::{
            DeviceStatus, DeviceStatusValue, PowerMode, PowerStatus,
        };
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let mut status: Envelope<DeviceStatus> = Envelope::from_json(include_bytes!(
            "../../../../test/fixtures/protocol/valid/status-with-capabilities.json"
        ))
        .unwrap();
        status.message_id = MessageId::from_uuid(uuid::Uuid::new_v4());
        status.sequence = Some(status.sequence.unwrap() + 1);
        status.data.status = DeviceStatusValue::Offline;
        status.data.reason = Some("sleeping".into());
        status.data.power = Some(Box::new(PowerStatus {
            mode: PowerMode::Battery,
            wake_interval_seconds: Some(60),
            expected_wake_ms: Some(u64::MAX),
            wake_reason: None,
            battery_mv: None,
            awake_ms: None,
        }));
        rhizo_storage::repo::ingest::persist_status(&db, &status, 1_000)
            .await
            .unwrap();
        let clock =
            rhizo_testkit::TestClock::new(Utc.timestamp_millis_opt(361_001).single().unwrap());
        let (client, _) = rumqttc::AsyncClient::new(
            rumqttc::MqttOptions::new("missed-wake-test", "127.0.0.1", 9),
            4,
        );
        let metrics = Metrics::new().unwrap();
        tick(&db, &clock, &client, &metrics).await.unwrap();
        let row = sqlx::query("SELECT connectivity_mode,missed_wake_count FROM devices")
            .fetch_one(db.pool())
            .await
            .unwrap();
        use sqlx::Row as _;
        assert_eq!(row.get::<String, _>("connectivity_mode"), "isolated");
        assert_eq!(row.get::<i64, _>("missed_wake_count"), 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM device_events WHERE kind='device_wake_missed'"
            )
            .fetch_one(db.pool())
            .await
            .unwrap(),
            1
        );
        tick(&db, &clock, &client, &metrics).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT missed_wake_count FROM devices")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            1
        );
        status.message_id = MessageId::from_uuid(uuid::Uuid::new_v4());
        status.data.boot_generation += 1;
        status.sequence = Some(1);
        status.data.status = DeviceStatusValue::Online;
        status.data.reason = None;
        rhizo_storage::repo::ingest::persist_status(&db, &status, 362_000)
            .await
            .unwrap();
        let row =
            sqlx::query("SELECT connectivity_mode,missed_wake_count,expected_wake_at FROM devices")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(row.get::<String, _>("connectivity_mode"), "connected");
        assert_eq!(row.get::<i64, _>("missed_wake_count"), 0);
        assert_eq!(row.get::<Option<i64>, _>("expected_wake_at"), None);
    }
}
