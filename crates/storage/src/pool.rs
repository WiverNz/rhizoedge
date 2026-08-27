//! SQLite connection ownership and operational pragmas.

use std::path::Path;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::StorageError;

/// Edge SQLite database handle.
#[derive(Clone)]
pub struct EdgeDb {
    pool: SqlitePool,
    path: Option<std::path::PathBuf>,
}

impl EdgeDb {
    /// Opens or creates a file database with ADR-004's pragmas.
    pub async fn connect(path: &Path) -> Result<Self, StorageError> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| StorageError::Database(e.to_string()))?;
        }
        let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
        let options = SqliteConnectOptions::from_str(&url)
            .map_err(|e| StorageError::Database(e.to_string()))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(5))
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .map_err(StorageError::from_sqlx)?;
        Ok(Self {
            pool,
            path: Some(path.to_path_buf()),
        })
    }

    /// Creates a shared-cache in-memory database for tests.
    pub async fn in_memory() -> Result<Self, StorageError> {
        let name = format!("rhizo-{}", uuid::Uuid::new_v4());
        let options =
            SqliteConnectOptions::from_str(&format!("sqlite:file:{name}?mode=memory&cache=shared"))
                .map_err(|e| StorageError::Database(e.to_string()))?
                .journal_mode(SqliteJournalMode::Memory)
                .synchronous(SqliteSynchronous::Normal)
                .busy_timeout(std::time::Duration::from_secs(5))
                .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .map_err(StorageError::from_sqlx)?;
        Ok(Self { pool, path: None })
    }

    /// Applies embedded forward-only migrations.
    pub async fn migrate(&self) -> Result<(), StorageError> {
        crate::migrate::run(self).await
    }
    /// Begins the single logical writer transaction.
    pub async fn begin(&self) -> Result<Transaction<'_, Sqlite>, StorageError> {
        self.pool.begin().await.map_err(StorageError::from_sqlx)
    }
    /// Read-only consumers use the pool.
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    /// Closes all pooled connections.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;
    #[tokio::test]
    async fn pragmas_apply_to_pooled_connections_and_transactions_work() {
        let db = EdgeDb::in_memory().await.unwrap();
        for _ in 0..2 {
            let mut c = db.pool.acquire().await.unwrap();
            assert_eq!(
                sqlx::query("PRAGMA foreign_keys")
                    .fetch_one(&mut *c)
                    .await
                    .unwrap()
                    .get::<i64, _>(0),
                1
            );
            assert_eq!(
                sqlx::query("PRAGMA busy_timeout")
                    .fetch_one(&mut *c)
                    .await
                    .unwrap()
                    .get::<i64, _>(0),
                5000
            );
        }
        let mut tx = db.begin().await.unwrap();
        sqlx::query("CREATE TABLE t(v INTEGER)")
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let mut tx = db.begin().await.unwrap();
        sqlx::query("INSERT INTO t VALUES(1)")
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.rollback().await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM t")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            0
        );
    }
}
