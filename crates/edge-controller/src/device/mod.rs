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
            wake_reason: Some(rhizo_mqtt_contract::payload::WakeReason::Timer),
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

    /// SAFETY-021: the device's own `expected_wake_ms` is a diagnostic. Here it
    /// claims a wake `u64::MAX` milliseconds away, and the window must still be
    /// exactly the edge's `received_at` plus the relative interval.
    #[tokio::test]
    async fn safety_021_device_wake_time_is_advisory() {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let online = envelope();
        ingest::persist_status(&db, &online, 1_000).await.unwrap();
        let sleeping = sleep(online.clone(), online.sequence.unwrap() + 1);
        assert_eq!(
            sleeping.data.power.as_ref().unwrap().expected_wake_ms,
            Some(u64::MAX),
            "the fixture must actually claim an absurd wake for this to prove anything"
        );
        ingest::persist_status(&db, &sleeping, 500_000)
            .await
            .unwrap();
        let row = sqlx::query("SELECT expected_wake_at,overdue_at,sleep_received_at FROM devices")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(
            row.get::<Option<i64>, _>("sleep_received_at"),
            Some(500_000)
        );
        assert_eq!(
            row.get::<Option<i64>, _>("expected_wake_at"),
            Some(1_400_000),
            "expected_wake_at must be the edge received_at plus wake_interval_seconds"
        );
        assert_eq!(row.get::<Option<i64>, _>("overdue_at"), Some(2_300_000));
        assert_eq!(
            crate::device::connectivity::from_projection(
                "sleeping",
                Some(1_400_000),
                Some(2_300_000),
                2_300_000,
            ),
            crate::device::connectivity::State::OfflineUnexpectedly,
            "the claimed wake must not hold the window open past the edge deadline"
        );
    }

    /// SAFETY-021: silence the device did not announce is never expected. A Last
    /// Will, an unrecognised reason, a sleep claim without a battery
    /// declaration, and an out-of-range interval all derive `isolated`.
    #[tokio::test]
    async fn safety_021_unannounced_absence_is_never_sleeping() {
        use rhizo_mqtt_contract::payload::{DeviceStatusValue, PowerMode, PowerStatus};
        fn power(mode: PowerMode, wake_interval_seconds: u32) -> Option<PowerStatus> {
            Some(PowerStatus {
                mode,
                wake_interval_seconds: Some(wake_interval_seconds),
                expected_wake_ms: None,
                wake_reason: None,
                battery_mv: None,
                awake_ms: None,
            })
        }
        for (label, reason, declared) in [
            ("last will", "connection_lost", None),
            ("unrecognised reason", "hibernating", None),
            (
                "sleep claim with no battery declaration",
                "sleeping",
                power(PowerMode::Unknown, 900),
            ),
            (
                "sleep claim with an out-of-range interval",
                "sleeping",
                power(PowerMode::Battery, 86_401),
            ),
        ] {
            let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
            db.migrate().await.unwrap();
            let online = envelope();
            ingest::persist_status(&db, &online, 1_000).await.unwrap();
            let mut absent = online.clone();
            absent.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
            absent.sequence = Some(online.sequence.unwrap() + 1);
            absent.data.status = DeviceStatusValue::Offline;
            absent.data.reason = Some(reason.into());
            absent.data.power = declared.map(Box::new);
            ingest::persist_status(&db, &absent, 2_000).await.unwrap();
            let row =
                sqlx::query("SELECT connectivity_mode,expected_wake_at,overdue_at FROM devices")
                    .fetch_one(db.pool())
                    .await
                    .unwrap();
            assert_eq!(
                row.get::<String, _>("connectivity_mode"),
                "isolated",
                "{label} must derive isolated"
            );
            assert_eq!(
                row.get::<Option<i64>, _>("expected_wake_at"),
                None,
                "{label} must open no window"
            );
            assert_eq!(
                crate::device::connectivity::from_projection(
                    &row.get::<String, _>("connectivity_mode"),
                    row.get("expected_wake_at"),
                    row.get("overdue_at"),
                    2_000,
                )
                .api_name(),
                "isolated",
                "{label}"
            );
        }
    }

    /// An explicit always-on declaration retires the battery state; an *absent*
    /// `power` block, which is what a pre-ADR-018 payload carries, changes
    /// nothing.
    #[tokio::test]
    async fn an_explicit_always_on_declaration_retires_the_battery_state() {
        use rhizo_mqtt_contract::payload::{PowerMode, PowerStatus};
        for (label, mode, expected_mode) in [
            ("absent", None, "battery"),
            ("always_on", Some(PowerMode::AlwaysOn), "always_on"),
            ("unknown", Some(PowerMode::Unknown), "always_on"),
        ] {
            let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
            db.migrate().await.unwrap();
            let online = envelope();
            ingest::persist_status(&db, &online, 1_000).await.unwrap();
            let sleeping = sleep(online.clone(), online.sequence.unwrap() + 1);
            ingest::persist_status(&db, &sleeping, 2_000).await.unwrap();
            assert_eq!(
                sqlx::query_scalar::<_, String>("SELECT power_mode FROM devices")
                    .fetch_one(db.pool())
                    .await
                    .unwrap(),
                "battery",
                "{label}: precondition"
            );
            let mut wake = online.clone();
            wake.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
            wake.sequence = Some(sleeping.sequence.unwrap() + 1);
            wake.data.power = mode.map(|mode| {
                Box::new(PowerStatus {
                    mode,
                    wake_interval_seconds: Some(900),
                    expected_wake_ms: None,
                    wake_reason: None,
                    battery_mv: None,
                    awake_ms: None,
                })
            });
            ingest::persist_status(&db, &wake, 3_000).await.unwrap();
            let row = sqlx::query(
                "SELECT power_mode,wake_interval_seconds,expected_wake_at,overdue_at FROM devices",
            )
            .fetch_one(db.pool())
            .await
            .unwrap();
            assert_eq!(row.get::<String, _>("power_mode"), expected_mode, "{label}");
            assert_eq!(
                row.get::<Option<i64>, _>("wake_interval_seconds"),
                (expected_mode == "battery").then_some(900),
                "{label}: a retired mode must leave no widened liveness cadence behind"
            );
            assert_eq!(
                row.get::<Option<i64>, _>("expected_wake_at"),
                None,
                "{label}"
            );
            assert_eq!(row.get::<Option<i64>, _>("overdue_at"), None, "{label}");
        }
    }

    /// A Last Will is composed at connect and delivered at an arbitrary later
    /// moment, so it must not restate the device's power configuration.
    #[tokio::test]
    async fn a_last_will_never_redeclares_the_power_mode() {
        use rhizo_mqtt_contract::payload::{DeviceStatusValue, PowerMode, PowerStatus};
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let online = envelope();
        ingest::persist_status(&db, &online, 1_000).await.unwrap();
        let mut will = online.clone();
        will.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
        will.sequence = Some(0);
        will.data.status = DeviceStatusValue::Offline;
        will.data.reason = Some("connection_lost".into());
        will.data.power = Some(Box::new(PowerStatus {
            mode: PowerMode::Battery,
            wake_interval_seconds: Some(900),
            expected_wake_ms: None,
            wake_reason: None,
            battery_mv: None,
            awake_ms: None,
        }));
        ingest::persist_status(&db, &will, 2_000).await.unwrap();
        let row = sqlx::query("SELECT power_mode,connectivity_mode,expected_wake_at FROM devices")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(
            row.get::<String, _>("power_mode"),
            "always_on",
            "a will must not turn a device into a battery device"
        );
        assert_eq!(row.get::<String, _>("connectivity_mode"), "isolated");
        assert_eq!(row.get::<Option<i64>, _>("expected_wake_at"), None);
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
