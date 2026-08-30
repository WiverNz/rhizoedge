use crate::{EdgeDb, StorageError};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations/edge");

/// Applies the embedded migrations, backing the database up first.
///
/// The queries here are deliberately *not* compile-time checked (M3-004). They
/// read `sqlite_master` and `_sqlx_migrations` — the migrator's own bookkeeping
/// table, which does not exist on a fresh database and is created by the very
/// call this function is about to make. A macro would have to describe a table
/// whose existence is the question being asked. They reference no application
/// table, so no schema change can invalidate them, and
/// [`tests::fresh_and_idempotent`] plus every integration test that opens a
/// database exercises this path on each run.
pub(crate) async fn run(db: &EdgeDb) -> Result<(), StorageError> {
    let migration_table_exists: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
    )
    .fetch_one(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    let applied = if migration_table_exists == 0 {
        0
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT coalesce(max(version),0) FROM _sqlx_migrations WHERE success=1",
        )
        .fetch_one(db.pool())
        .await
        .map_err(StorageError::from_sqlx)?
    };
    let target = MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0);
    if applied < target
        && let Some(path) = db
            .path()
            .filter(|p| p.exists() && p.metadata().is_ok_and(|m| m.len() > 0))
    {
        sqlx::query("PRAGMA wal_checkpoint(FULL)")
            .execute(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?;
        std::fs::copy(path, path.with_extension("sqlite.pre-migration.bak"))
            .map_err(|e| StorageError::Migration(e.to_string()))?;
    }
    MIGRATOR
        .run(db.pool())
        .await
        .map_err(|e| StorageError::Migration(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn fresh_and_idempotent() {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        db.migrate().await.unwrap();
        let tables: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='measurements'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(tables, 1);
    }

    /// The canonical pre-release baseline, plus every forward-only migration
    /// applied after it.
    ///
    /// `0001_initial.sql` was squashed while no release existed
    /// (`versioning-policy.md` §4). M6 is additive and arrives as
    /// `0002_irrigation_control.sql` rather than by editing the baseline, so an
    /// existing development database upgrades in place — and the backup path in
    /// [`run`] is exercised by a real version change rather than only by a fresh
    /// create.
    #[tokio::test]
    async fn canonical_baseline_contains_the_final_schema() {
        use sqlx::Row as _;

        assert_eq!(MIGRATOR.iter().count(), 2);
        let versions: Vec<i64> = MIGRATOR.iter().map(|m| m.version).collect();
        assert_eq!(versions, [1, 2]);
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();

        let tables = sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != '_sqlx_migrations' ORDER BY name",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(
            tables,
            [
                "actuator_bindings",
                "actuator_states",
                "command_intents",
                "command_results",
                "commands",
                "device_capabilities",
                "device_events",
                "device_isolation_periods",
                "devices",
                "history_gaps",
                "irrigation_state",
                "measurement_policies",
                "measurements",
                "offline_policies",
                "pending_cloud_events",
                "plant_dry_state",
                "plant_events",
                "plant_profiles",
                "plant_recommendations",
                "plant_state_current",
                "plant_threshold_state",
                "plants",
                "processed_messages",
                "quarantined_messages",
                "replay_progress",
                "sensor_bindings",
                "sensor_stuck_state",
                "watering_events",
            ]
        );

        let indexes = sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_schema WHERE type='index' AND sql IS NOT NULL ORDER BY name",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(
            indexes,
            [
                "idx_binding_plant",
                "idx_commands_open",
                "idx_devevents_device_time",
                "idx_devevents_replay",
                "idx_device_isolation_periods",
                "idx_devices_sleep_deadline",
                "idx_intents_open",
                "idx_meas_batch",
                "idx_meas_lookup",
                "idx_meas_time",
                "idx_outbox_ready",
                "idx_plant_events_time",
                "idx_plants_live",
                "idx_processed_received",
                "idx_reco_plant_time",
                "idx_watering_plant_time",
                "uq_binding_control",
                "uq_command_result_command",
                "uq_measurement_batch_sample",
                "uq_open_water_intent",
            ]
        );

        async fn columns(db: &EdgeDb, table: &str) -> Vec<String> {
            sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
                .bind(table)
                .fetch_all(db.pool())
                .await
                .unwrap()
        }
        assert_eq!(
            columns(&db, "devices").await,
            [
                "device_id",
                "name",
                "firmware_version",
                "boot_id",
                "last_sequence",
                "status",
                "clock_synced",
                "last_seen_at",
                "desired_config_version",
                "applied_config_version",
                "created_at",
                "status_json",
                "status_boot_generation",
                "status_sequence",
                "status_lwt_message_id",
                "protocol_version",
                "uptime_ms",
                "free_heap_bytes",
                "rssi_dbm",
                "sensors_json",
                "telemetry_interval_seconds",
                "drift_since",
                "connectivity_mode",
                "isolation_started_at",
                "last_time_sync_at",
                "power_mode",
                "wake_interval_seconds",
                "sleep_received_at",
                "expected_wake_at",
                "overdue_at",
                "missed_wake_count"
            ]
        );
        assert_eq!(
            columns(&db, "plants").await,
            [
                "plant_id",
                "profile_id",
                "name",
                "species",
                "pot_volume_ml",
                "soil_type",
                "auto_watering_enabled",
                "lockout_reason",
                "lockout_since",
                "created_at",
                "deleted_at",
                "applied_preset_id",
                "applied_catalogue_version",
                "lockout_cleared_by",
                "lockout_cleared_at",
                "lockout_until"
            ]
        );
        assert_eq!(
            columns(&db, "measurements").await,
            [
                "id",
                "device_id",
                "sensor_id",
                "point",
                "kind",
                "value_num",
                "value_bool",
                "unit",
                "quality",
                "calibration_ref",
                "received_at",
                "device_time_ms",
                "boot_id",
                "sequence",
                "batch_id",
                "origin",
                "source_message_id",
                "sample_index"
            ]
        );

        let replay = sqlx::query(
            "SELECT \"notnull\" AS is_not_null,dflt_value,pk FROM pragma_table_info('replay_progress') WHERE name='through_device_seq'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(replay.get::<i64, _>("is_not_null"), 0);
        assert_eq!(replay.get::<Option<String>, _>("dflt_value"), None);
        assert_eq!(replay.get::<i64, _>("pk"), 0);

        assert_eq!(
            columns(&db, "command_intents").await,
            [
                "intent_id",
                "plant_id",
                "device_id",
                "kind",
                "requested_ml",
                "mode",
                "created_at",
                "intent_expires_at",
                "expected_delivery_after",
                "state",
                "command_id",
                "refusal_reason",
                "settled_at",
                "updated_at"
            ]
        );
        // An intent is not a command: `command_id` is nullable, and `commands`
        // gained no column (M6-022).
        let intent_command_id = sqlx::query(
            "SELECT \"notnull\" AS is_not_null FROM pragma_table_info('command_intents') WHERE name='command_id'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(intent_command_id.get::<i64, _>("is_not_null"), 0);
        assert_eq!(
            columns(&db, "commands").await,
            [
                "command_id",
                "device_id",
                "plant_id",
                "kind",
                "requested_ml",
                "mode",
                "issued_at",
                "expires_at",
                "status",
                "published_at",
                "settled_at",
                "reason"
            ],
            "M6 adds no column to `commands`"
        );
        assert_eq!(
            columns(&db, "irrigation_state").await,
            [
                "plant_id",
                "state",
                "state_since",
                "doses_this_cycle",
                "cycle_started_at",
                "last_cycle_completed_at",
                "wait_until",
                "active_command_id",
                "updated_at",
                "pre_dose_vwc",
                "pre_dose_grams"
            ]
        );

        for (name, fragment) in [
            ("uq_binding_control", "WHERE role='control'"),
            (
                "uq_measurement_batch_sample",
                "WHERE sample_index IS NOT NULL",
            ),
            (
                "idx_devices_sleep_deadline",
                "WHERE connectivity_mode = 'sleeping'",
            ),
            (
                "uq_open_water_intent",
                "WHERE state = 'pending_for_device_wake' AND kind = 'water'",
            ),
        ] {
            let sql: String = sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE name=?")
                .bind(name)
                .fetch_one(db.pool())
                .await
                .unwrap();
            assert!(
                sql.contains(fragment),
                "{name} lost its partial predicate: {sql}"
            );
        }

        let cascades: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM (SELECT on_delete FROM pragma_foreign_key_list('sensor_bindings') UNION ALL SELECT on_delete FROM pragma_foreign_key_list('actuator_bindings') UNION ALL SELECT on_delete FROM pragma_foreign_key_list('measurement_policies') UNION ALL SELECT on_delete FROM pragma_foreign_key_list('offline_policies') UNION ALL SELECT on_delete FROM pragma_foreign_key_list('plant_dry_state') UNION ALL SELECT on_delete FROM pragma_foreign_key_list('plant_state_current') UNION ALL SELECT on_delete FROM pragma_foreign_key_list('plant_threshold_state')) WHERE on_delete='CASCADE'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(cascades, 7);
    }
}
