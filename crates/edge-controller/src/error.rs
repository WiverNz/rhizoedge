//! Controller failures.
#![allow(missing_docs)]
use rhizo_telemetry::{Classify, FailureKind};
/// MQTT/pipeline error.
#[derive(Debug, thiserror::Error)]
pub enum EdgeError {
    #[error("MQTT unavailable: {0}")]
    Mqtt(String),
    #[error("payload rejected: {0}")]
    Decode(String),
    #[error("storage: {0}")]
    Storage(#[from] rhizo_storage::StorageError),
    #[error("supervised task `{0}` failed")]
    Task(String),
}
impl Classify for EdgeError {
    fn classify(&self) -> FailureKind {
        match self {
            Self::Mqtt(_) => FailureKind::Transient,
            Self::Decode(_) => FailureKind::Permanent,
            Self::Storage(e) => e.classify(),
            Self::Task(_) => FailureKind::Fatal,
        }
    }
}
#[cfg(test)]
mod classify {
    use super::*;
    #[test]
    fn variants() {
        assert_eq!(
            EdgeError::Mqtt("x".into()).classify(),
            FailureKind::Transient
        );
        assert_eq!(
            EdgeError::Decode("x".into()).classify(),
            FailureKind::Permanent
        );
        assert_eq!(EdgeError::Task("x".into()).classify(), FailureKind::Fatal);
    }
    /// `Storage` has no kind of its own — it defers, so a storage variant
    /// reclassified in `rhizo-storage` changes the pipeline's behaviour too.
    #[test]
    fn storage_defers_to_the_storage_classification() {
        for storage in [
            rhizo_storage::StorageError::Busy("x".into()),
            rhizo_storage::StorageError::Constraint("x".into()),
            rhizo_storage::StorageError::Full("x".into()),
        ] {
            let expected = storage.classify();
            assert_eq!(EdgeError::Storage(storage).classify(), expected);
        }
    }
}
