use crate::{EdgeDb, StorageError};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations/edge");

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
}
