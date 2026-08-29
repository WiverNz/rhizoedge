//! Storage failures and their retry classification.

use rhizo_telemetry::{Classify, FailureKind};

/// A storage operation failure.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// SQLite reported a transient busy/locked condition.
    #[error("SQLite is busy: {0}")]
    Busy(String),
    /// The database cannot accept more data.
    #[error("SQLite storage is full: {0}")]
    Full(String),
    /// A row violated a schema constraint.
    #[error("SQLite constraint violation: {0}")]
    Constraint(String),
    /// Migration or backup failed and startup cannot safely continue.
    #[error("migration failed: {0}")]
    Migration(String),
    /// Other database failure.
    #[error("SQLite failure: {0}")]
    Database(String),
    /// JSON serialization failed.
    #[error("serialization failure: {0}")]
    Serialization(String),
}

impl StorageError {
    pub(crate) fn from_sqlx(error: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db) = &error {
            // SQLite reports *extended* result codes: the low byte is the
            // primary code and the high bits say which flavour of it. A foreign
            // key violation is 787, which is `19 | (3 << 8)` — the same
            // `SQLITE_CONSTRAINT` as a unique violation. Listing the extended
            // codes one at a time is how 787 came to be classified `Fatal`
            // rather than `Permanent`, which would have made a caller retry a
            // write that can never succeed. Masking to the primary code covers
            // every flavour, including the ones SQLite has not added yet.
            let primary = db
                .code()
                .as_deref()
                .and_then(|code| code.parse::<u32>().ok())
                .map(|code| code & 0xFF);
            return match primary {
                Some(5 | 6) => Self::Busy(error.to_string()),
                Some(13) => Self::Full(error.to_string()),
                Some(19) => Self::Constraint(error.to_string()),
                _ => Self::Database(error.to_string()),
            };
        }
        Self::Database(error.to_string())
    }
}

impl Classify for StorageError {
    fn classify(&self) -> FailureKind {
        match self {
            Self::Busy(_) => FailureKind::Transient,
            Self::Full(_) | Self::Migration(_) | Self::Database(_) => FailureKind::Fatal,
            Self::Constraint(_) | Self::Serialization(_) => FailureKind::Permanent,
        }
    }
}

#[cfg(test)]
mod classify {
    use super::*;
    #[test]
    fn every_variant() {
        assert_eq!(
            StorageError::Busy("x".into()).classify(),
            FailureKind::Transient
        );
        assert_eq!(
            StorageError::Full("x".into()).classify(),
            FailureKind::Fatal
        );
        assert_eq!(
            StorageError::Constraint("x".into()).classify(),
            FailureKind::Permanent
        );
        assert_eq!(
            StorageError::Migration("x".into()).classify(),
            FailureKind::Fatal
        );
        assert_eq!(
            StorageError::Database("x".into()).classify(),
            FailureKind::Fatal
        );
        assert_eq!(
            StorageError::Serialization("x".into()).classify(),
            FailureKind::Permanent
        );
    }
}
