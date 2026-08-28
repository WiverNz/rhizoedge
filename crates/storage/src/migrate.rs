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

    /// An existing M4 database must upgrade in place, not be rebuilt.
    ///
    /// `0006` is the first migration that runs against installations that are
    /// already carrying real device history, so the interesting question is not
    /// whether a fresh database ends up with the columns -- `fresh_and_idempotent`
    /// covers that -- but whether a *populated* pre-battery database keeps its
    /// rows and lands on defaults that behave exactly as it did before. Every
    /// device that existed before ADR-018 is an always-on device, and it must
    /// stay one without anybody writing to it.
    #[tokio::test]
    async fn upgrading_a_populated_pre_battery_database_preserves_always_on() {
        use std::borrow::Cow;
        /// The schema as M4 originally shipped it: everything up to `0005`.
        fn pre_battery() -> sqlx::migrate::Migrator {
            sqlx::migrate::Migrator {
                migrations: Cow::Owned(
                    MIGRATOR
                        .iter()
                        .filter(|m| m.version <= 5)
                        .cloned()
                        .collect::<Vec<_>>(),
                ),
                ..sqlx::migrate::Migrator::DEFAULT
            }
        }
        let db = EdgeDb::in_memory().await.unwrap();
        let old = pre_battery();
        assert_eq!(
            old.iter().count(),
            5,
            "the pre-battery schema is 0001..0005"
        );
        old.run(db.pool()).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM pragma_table_info('devices') WHERE name='power_mode'"
            )
            .fetch_one(db.pool())
            .await
            .unwrap(),
            0,
            "precondition: the battery columns must not exist yet"
        );
        sqlx::query(
            "INSERT INTO devices(device_id,created_at,status,last_seen_at,connectivity_mode)              VALUES('plant-node-01',1000,'online',1000,'connected')",
        )
        .execute(db.pool())
        .await
        .unwrap();

        db.migrate().await.unwrap();

        let row = sqlx::query(
            "SELECT status,last_seen_at,connectivity_mode,power_mode,wake_interval_seconds,             sleep_received_at,expected_wake_at,overdue_at,missed_wake_count FROM devices              WHERE device_id='plant-node-01'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        use sqlx::Row as _;
        assert_eq!(row.get::<String, _>("status"), "online");
        assert_eq!(row.get::<Option<i64>, _>("last_seen_at"), Some(1000));
        assert_eq!(row.get::<String, _>("connectivity_mode"), "connected");
        assert_eq!(
            row.get::<String, _>("power_mode"),
            "always_on",
            "an upgraded device must default to the behaviour it already had"
        );
        assert_eq!(row.get::<i64, _>("missed_wake_count"), 0);
        for column in [
            "wake_interval_seconds",
            "sleep_received_at",
            "expected_wake_at",
            "overdue_at",
        ] {
            assert_eq!(
                row.get::<Option<i64>, _>(column),
                None,
                "{column} must be absent, so no upgraded device has an open window"
            );
        }
        // Re-running is a no-op, and the upgraded database matches a fresh one.
        db.migrate().await.unwrap();
        let fresh = EdgeDb::in_memory().await.unwrap();
        fresh.migrate().await.unwrap();
        let columns = |db: &EdgeDb| {
            let pool = db.pool().clone();
            async move {
                sqlx::query_scalar::<_, String>(
                    "SELECT name FROM pragma_table_info('devices') ORDER BY name",
                )
                .fetch_all(&pool)
                .await
                .unwrap()
            }
        };
        assert_eq!(
            columns(&db).await,
            columns(&fresh).await,
            "an upgraded database and a fresh one must have the same devices table"
        );
    }
}
