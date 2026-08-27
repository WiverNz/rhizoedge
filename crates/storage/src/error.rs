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
            return match db.code().as_deref() {
                Some("5" | "6") => Self::Busy(error.to_string()),
                Some("13") => Self::Full(error.to_string()),
                Some("19" | "1555" | "2067") => Self::Constraint(error.to_string()),
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
