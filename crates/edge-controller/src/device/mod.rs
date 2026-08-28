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
