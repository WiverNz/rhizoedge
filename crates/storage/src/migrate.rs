use crate::{EdgeDb, StorageError};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations/edge");

/// Applies the embedded migrations, backing the database up first.
///
/// The backup fires whenever `applied < target` and the database file is
/// present and non-empty. **That includes a fresh create**: `EdgeDb::connect`
/// sets `journal_mode = WAL`, which writes the 4 KiB header before this
/// function runs, so a brand-new database is non-empty and gets a 4 KiB backup
/// of nothing. Harmless — it is overwritten by the next real one — but it is
/// what happens, and
/// [`tests::a_fresh_file_database_is_backed_up_before_its_first_migration`]
/// holds that to be true so the next reader does not have to guess.
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

    /// The pre-migration backup on a **fresh** file database.
    ///
    /// Written because the obvious assumption — "a fresh database is absent or
    /// empty, so the backup is skipped" — is wrong, and was stated as fact in
    /// this file and in ADR-004 until this test was written.
    /// `EdgeDb::connect` applies `journal_mode = WAL`, and switching journal
    /// mode writes the SQLite header, so by the time [`run`] looks the file
    /// exists and is 4 KiB.
    ///
    /// The behaviour is left alone: a 4 KiB copy of an empty database on first
    /// start is harmless, and tightening the condition risks skipping a backup
    /// that matters. This test exists to keep the *documentation* honest, and
    /// will fail if the pragmas or the backup condition change — at which point
    /// the comments above need revisiting too.
    #[tokio::test]
    async fn a_fresh_file_database_is_backed_up_before_its_first_migration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.sqlite");

        let db = EdgeDb::connect(&path).await.unwrap();
        assert!(path.exists(), "connect creates the file");
        assert!(
            std::fs::metadata(&path).unwrap().len() > 0,
            "WAL journal mode writes the header, so the file is not empty"
        );

        db.migrate().await.unwrap();

        let backup = path.with_extension("sqlite.pre-migration.bak");
        assert!(
            backup.exists(),
            "a fresh create still satisfies `applied < target` with a non-empty file"
        );
    }

    /// The canonical baseline plus its forward migrations hold the **whole**
    /// schema.
    ///
    /// **The migration-count assertion is deliberate.** It fails the moment
    /// someone adds a file, which is the right moment to ask whether the change
    /// belongs in a forward migration or in the baseline
    /// (`versioning-policy.md` §4). Either way the decision gets made rather
    /// than defaulted into. `0002_plant_lockouts` is forward on purpose: it
    /// carries existing alarms out of `plants.lockout_reason`, and a rewritten
    /// baseline would have meant recreating the database and losing them.
    ///
    /// The rest pins the schema itself — the table set, the index set, the
    /// column order of every table, the partial-index predicates, and the
    /// cascade count — so an edit to the baseline has to be deliberate too.
    ///
    /// The upgrade from `0001` is backed up like any other — see
    /// [`tests::a_v1_database_backs_up_and_carries_every_alarm_into_plant_lockouts`].
    /// The backup also runs on a fresh create — see
    /// [`tests::a_fresh_file_database_is_backed_up_before_its_first_migration`],
    /// which pins what actually happens rather than what is easy to assume.
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
                "cloud_sync_settings",
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
                "plant_lockouts",
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

    /// Migration `0002` against a real `0001` database holding every legacy
    /// lockout shape, fail-closed:
    ///
    /// - every explicit-clear reason, an unrecognised one, and a blank one
    ///   become latches (the blank one named `unknown`), keeping their `since`;
    /// - a held reason becomes a hold whether its deadline has passed or not
    ///   (the SQL cannot see the edge clock; the reader can);
    /// - a known derived reason with no deadline is not recorded, because the
    ///   next pass re-derives it from live inputs;
    /// - a missing `since` falls back to the plant's creation time instead of
    ///   dropping the row; a soft-deleted plant is migrated too.
    ///
    /// Nothing in `plants` changes, every migrated row is announced in
    /// `plant_events` (an unrecognised one as a warning), the upgrade is backed
    /// up first, and reopening and migrating again changes nothing.
    #[tokio::test]
    async fn a_v1_database_backs_up_and_carries_every_alarm_into_plant_lockouts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.sqlite");
        /// `(plant_id, lockout_reason, lockout_since, lockout_until, deleted_at)`.
        type LegacyRow = (
            &'static str,
            Option<&'static str>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        );
        let legacy: [LegacyRow; 14] = [
            ("p-leak", Some("leak"), Some(10), None, None),
            ("p-pump", Some("pump_fault"), Some(11), None, None),
            (
                "p-nodelivery",
                Some("no_delivery_detected"),
                Some(12),
                None,
                None,
            ),
            (
                "p-maxdoses",
                Some("max_doses_reached"),
                Some(13),
                None,
                None,
            ),
            ("p-unknown", Some("unknown"), Some(14), None, None),
            ("p-garbage", Some("flood_v9"), Some(15), Some(99), None),
            ("p-blank", Some(" "), Some(16), None, None),
            ("p-leak-held", Some("leak"), Some(17), Some(5), None),
            (
                "p-hold-active",
                Some("uncertain"),
                Some(18),
                Some(i64::MAX),
                None,
            ),
            (
                "p-hold-expired",
                Some("clock_unsynced"),
                Some(19),
                Some(1),
                None,
            ),
            ("p-derived", Some("tank_low"), Some(20), None, None),
            ("p-no-since", Some("leak"), None, None, None),
            (
                "p-deleted",
                Some("max_doses_reached"),
                Some(22),
                None,
                Some(5),
            ),
            ("p-clear", None, None, None, None),
        ];
        {
            let db = EdgeDb::connect(&path).await.unwrap();
            MIGRATOR.run_to(1, db.pool()).await.unwrap();
            for (plant, reason, since, until, deleted) in legacy {
                sqlx::query(
                    "INSERT INTO plants(plant_id,name,created_at,lockout_reason,lockout_since,lockout_until,deleted_at) \
                     VALUES(?,?,7,?,?,?,?)",
                )
                .bind(plant)
                .bind(plant)
                .bind(reason)
                .bind(since)
                .bind(until)
                .bind(deleted)
                .execute(db.pool())
                .await
                .unwrap();
            }
            db.migrate().await.unwrap();
            db.pool().close().await;
        }
        assert!(
            path.with_extension("sqlite.pre-migration.bak").exists(),
            "the upgrade is backed up before it runs"
        );

        let expected: Vec<(String, String, String, i64, Option<i64>)> = [
            ("p-blank", "unknown", "latch", 16, None),
            ("p-deleted", "max_doses_reached", "latch", 22, None),
            ("p-garbage", "flood_v9", "latch", 15, None),
            ("p-hold-active", "uncertain", "hold", 18, Some(i64::MAX)),
            ("p-hold-expired", "clock_unsynced", "hold", 19, Some(1)),
            ("p-leak", "leak", "latch", 10, None),
            ("p-leak-held", "leak", "latch", 17, None),
            ("p-maxdoses", "max_doses_reached", "latch", 13, None),
            ("p-no-since", "leak", "latch", 7, None),
            ("p-nodelivery", "no_delivery_detected", "latch", 12, None),
            ("p-pump", "pump_fault", "latch", 11, None),
            ("p-unknown", "unknown", "latch", 14, None),
        ]
        .into_iter()
        .map(|(p, r, k, s, u)| (p.to_owned(), r.to_owned(), k.to_owned(), s, u))
        .collect();

        for _ in 0..2 {
            let db = EdgeDb::connect(&path).await.unwrap();
            db.migrate().await.unwrap();
            let rows: Vec<(String, String, String, i64, Option<i64>)> = sqlx::query_as(
                "SELECT plant_id,reason,kind,since,hold_until FROM plant_lockouts ORDER BY plant_id",
            )
            .fetch_all(db.pool())
            .await
            .unwrap();
            assert_eq!(rows, expected);

            // The legacy column is untouched: it stays authoritative until the
            // cutover.
            let column: Option<String> =
                sqlx::query_scalar("SELECT lockout_reason FROM plants WHERE plant_id='p-derived'")
                    .fetch_one(db.pool())
                    .await
                    .unwrap();
            assert_eq!(column.as_deref(), Some("tank_low"));

            let announced: Vec<(String, String)> = sqlx::query_as(
                "SELECT plant_id,severity FROM plant_events WHERE kind='lockout_migrated' ORDER BY plant_id",
            )
            .fetch_all(db.pool())
            .await
            .unwrap();
            assert_eq!(announced.len(), expected.len(), "one record per row, once");
            let warned: Vec<&str> = announced
                .iter()
                .filter(|(_, severity)| severity == "warning")
                .map(|(plant, _)| plant.as_str())
                .collect();
            assert_eq!(warned, ["p-blank", "p-garbage", "p-unknown"]);

            // What is active depends on the reader's clock, never the SQL's.
            let active = crate::repo::lockout::active(&db, "p-hold-expired", 1_000)
                .await
                .unwrap();
            assert!(active.is_empty(), "an expired hold blocks nothing");
            let active = crate::repo::lockout::active(&db, "p-hold-active", 1_000)
                .await
                .unwrap();
            assert_eq!(active.len(), 1);
            let active = crate::repo::lockout::active(&db, "p-deleted", 1_000)
                .await
                .unwrap();
            assert!(
                active.is_empty(),
                "a deleted plant is migrated but not reported"
            );
            db.pool().close().await;
        }
    }

    /// The backfill is idempotent even if `0002` runs again — for example
    /// after `_sqlx_migrations` is lost — and a re-run never overwrites a
    /// record written since: an original `since`, an extended hold, and a
    /// latch the column no longer shows all survive.
    #[tokio::test]
    async fn rerunning_the_0002_backfill_changes_nothing() {
        let db = EdgeDb::in_memory().await.unwrap();
        MIGRATOR.run_to(1, db.pool()).await.unwrap();
        sqlx::query(
            "INSERT INTO plants(plant_id,name,created_at,lockout_reason,lockout_since,lockout_until) \
             VALUES('p','p',0,'uncertain',5,100)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        db.migrate().await.unwrap();
        crate::repo::lockout::latch(&db, "p", "leak", 50)
            .await
            .unwrap();
        crate::repo::lockout::hold(&db, "p", "uncertain", 900, 60)
            .await
            .unwrap();
        let before: Vec<(String, String, i64, Option<i64>)> = sqlx::query_as(
            "SELECT reason,kind,since,hold_until FROM plant_lockouts ORDER BY reason",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();

        let rerun = include_str!("../../../migrations/edge/0002_plant_lockouts.sql").replace(
            "CREATE TABLE plant_lockouts",
            "CREATE TABLE IF NOT EXISTS plant_lockouts",
        );
        // The repository's own migration file, with no external input in it.
        sqlx::raw_sql(sqlx::AssertSqlSafe(rerun))
            .execute(db.pool())
            .await
            .unwrap();

        let after: Vec<(String, String, i64, Option<i64>)> = sqlx::query_as(
            "SELECT reason,kind,since,hold_until FROM plant_lockouts ORDER BY reason",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(after, before);
        assert_eq!(
            after,
            [
                ("leak".into(), "latch".into(), 50, None),
                ("uncertain".into(), "hold".into(), 5, Some(900)),
            ]
        );
    }
}
