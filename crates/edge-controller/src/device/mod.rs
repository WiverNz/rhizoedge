//! Device registry projections and timer-driven health.
pub mod capabilities;
pub mod connectivity;
pub mod health;

#[cfg(test)]
mod status {
    use rhizo_storage::repo::ingest::{self, Dedup};
    use sqlx::Row as _;

    fn envelope() -> rhizo_mqtt_contract::Envelope<rhizo_mqtt_contract::payload::DeviceStatus> {
        rhizo_mqtt_contract::Envelope::from_json(include_bytes!(
            "../../../../test/fixtures/protocol/valid/status-with-capabilities.json"
        ))
        .unwrap()
    }
    #[tokio::test]
    async fn accepted_status_updates_registry_and_old_status_does_not() {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let current = envelope();
        assert_eq!(
            ingest::persist_status(&db, &current, 10_000).await.unwrap(),
            Dedup::New
        );
        let row=sqlx::query("SELECT status,last_seen_at,firmware_version FROM devices WHERE device_id='plant-node-01'").fetch_one(db.pool()).await.unwrap();
        assert_eq!(row.get::<String, _>("status"), "online");
        assert_eq!(row.get::<Option<i64>, _>("last_seen_at"), Some(10_000));
        let mut old = current.clone();
        old.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
        old.sequence = Some(11);
        assert_eq!(
            ingest::persist_status(&db, &old, 20_000).await.unwrap(),
            Dedup::Duplicate
        );
        assert_eq!(
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT last_seen_at FROM devices WHERE device_id='plant-node-01'"
            )
            .fetch_one(db.pool())
            .await
            .unwrap(),
            Some(10_000)
        );
    }

    fn sleep(
        mut e: rhizo_mqtt_contract::Envelope<rhizo_mqtt_contract::payload::DeviceStatus>,
        sequence: u64,
    ) -> rhizo_mqtt_contract::Envelope<rhizo_mqtt_contract::payload::DeviceStatus> {
        use rhizo_mqtt_contract::payload::{DeviceStatusValue, PowerMode, PowerStatus};
        e.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
        e.sequence = Some(sequence);
        e.data.status = DeviceStatusValue::Offline;
        e.data.reason = Some("sleeping".into());
        e.data.power = Some(Box::new(PowerStatus {
            mode: PowerMode::Battery,
            wake_interval_seconds: Some(900),
            expected_wake_ms: Some(u64::MAX),
            wake_reason: Some("timer".into()),
            battery_mv: None,
            awake_ms: None,
        }));
        e
    }

    #[tokio::test]
    async fn intentional_sleep_duplicate_lwt_and_wake_are_safe() {
        use rhizo_mqtt_contract::payload::DeviceStatusValue;
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let online = envelope();
        ingest::persist_status(&db, &online, 1_000).await.unwrap();
        let sleeping = sleep(online.clone(), online.sequence.unwrap() + 1);
        assert_eq!(
            ingest::persist_status(&db, &sleeping, 2_000).await.unwrap(),
            Dedup::New
        );
        let row = sqlx::query(
            "SELECT connectivity_mode,last_seen_at,expected_wake_at,overdue_at FROM devices",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("connectivity_mode"), "sleeping");
        assert_eq!(row.get::<Option<i64>, _>("last_seen_at"), Some(1_000));
        assert_eq!(row.get::<Option<i64>, _>("expected_wake_at"), Some(902_000));
        assert_eq!(row.get::<Option<i64>, _>("overdue_at"), Some(1_802_000));
        assert_eq!(
            ingest::persist_status(&db, &sleeping, 9_999).await.unwrap(),
            Dedup::Duplicate
        );

        let mut lwt = sleeping.clone();
        lwt.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
        lwt.sequence = Some(0);
        lwt.data.reason = Some("connection_lost".into());
        assert_eq!(
            ingest::persist_status(&db, &lwt, 3_000).await.unwrap(),
            Dedup::New
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT connectivity_mode FROM devices")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            "sleeping"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM device_events WHERE kind='offline'")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            0
        );

        let mut wake = sleeping.clone();
        wake.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
        wake.sequence = Some(sleeping.sequence.unwrap() + 1);
        wake.data.boot_generation += 1;
        wake.data.status = DeviceStatusValue::Online;
        wake.data.reason = None;
        assert_eq!(
            ingest::persist_status(&db, &wake, 4_000).await.unwrap(),
            Dedup::New
        );
        let row = sqlx::query(
            "SELECT connectivity_mode,last_seen_at,expected_wake_at,missed_wake_count FROM devices",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("connectivity_mode"), "connected");
        assert_eq!(row.get::<Option<i64>, _>("last_seen_at"), Some(4_000));
        assert_eq!(row.get::<Option<i64>, _>("expected_wake_at"), None);
        let mut delayed_old_lwt = lwt;
        delayed_old_lwt.message_id =
            rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
        assert_eq!(
            ingest::persist_status(&db, &delayed_old_lwt, 5_000)
                .await
                .unwrap(),
            Dedup::Duplicate
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT connectivity_mode FROM devices")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            "connected"
        );
    }

    #[tokio::test]
    async fn always_on_unexpected_disconnect_remains_offline() {
        use rhizo_mqtt_contract::payload::DeviceStatusValue;
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let online = envelope();
        ingest::persist_status(&db, &online, 1_000).await.unwrap();
        let mut lwt = online;
        lwt.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
        lwt.sequence = Some(0);
        lwt.data.status = DeviceStatusValue::Offline;
        lwt.data.reason = Some("connection_lost".into());
        ingest::persist_status(&db, &lwt, 2_000).await.unwrap();
        let row = sqlx::query("SELECT connectivity_mode,power_mode,last_seen_at FROM devices")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("connectivity_mode"), "isolated");
        assert_eq!(row.get::<String, _>("power_mode"), "always_on");
        assert_eq!(row.get::<Option<i64>, _>("last_seen_at"), Some(1_000));
    }

    #[tokio::test]
    async fn always_on_clean_shutdown_keeps_m4_reconciling_behavior() {
        use rhizo_mqtt_contract::payload::DeviceStatusValue;
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let mut shutdown = envelope();
        ingest::persist_status(&db, &shutdown, 1_000).await.unwrap();
        shutdown.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
        shutdown.sequence = Some(shutdown.sequence.unwrap() + 1);
        shutdown.data.status = DeviceStatusValue::Offline;
        shutdown.data.reason = Some("shutdown".into());
        ingest::persist_status(&db, &shutdown, 2_000).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT connectivity_mode FROM devices")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            "reconciling"
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT power_mode FROM devices")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            "always_on"
        );
    }
}

#[cfg(test)]
mod registration {
    #[tokio::test]
    async fn auto_registration_never_creates_a_plant() {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let e = rhizo_mqtt_contract::Envelope::from_json(include_bytes!(
            "../../../../test/fixtures/protocol/valid/status-with-capabilities.json"
        ))
        .unwrap();
        rhizo_storage::repo::ingest::persist_status(&db, &e, 1)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM plants")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM device_events WHERE kind='device_registered'"
            )
            .fetch_one(db.pool())
            .await
            .unwrap(),
            1
        );
    }
}

#[cfg(test)]
mod sensors {
    #[tokio::test]
    async fn absent_and_unhealthy_are_preserved_as_distinct_fields() {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let mut e: rhizo_mqtt_contract::Envelope<rhizo_mqtt_contract::payload::DeviceStatus> =
            rhizo_mqtt_contract::Envelope::from_json(include_bytes!(
                "../../../../test/fixtures/protocol/valid/status-with-capabilities.json"
            ))
            .unwrap();
        e.data.capabilities.sensors[0].present = false;
        e.data.capabilities.sensors[1].healthy = false;
        rhizo_storage::repo::ingest::persist_status(&db, &e, 1)
            .await
            .unwrap();
        let json: String = sqlx::query_scalar("SELECT sensors_json FROM devices")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert!(json.contains("\"present\":false"));
        assert!(json.contains("\"healthy\":false"));
    }
}

#[cfg(test)]
mod drift {
    #[test]
    fn grace_is_exactly_two_intervals() {
        assert_eq!(2_i64.saturating_mul(300_000), 600_000);
    }
}
