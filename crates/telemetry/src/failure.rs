//! Failure classification.
//!
//! [ADR-014](../../../../docs/adr/014-failure-and-retry-policy.md) requires
//! every failure in the system to be classified as exactly one of
//! [`FailureKind::Transient`], [`FailureKind::Permanent`], or
//! [`FailureKind::Fatal`], and requires that classification to be a *function*
//! rather than scattered `match` arms at each of the five retry sites.

use core::fmt;

/// How a failure must be handled.
///
/// The classification determines behaviour, not severity: a `Permanent`
/// failure may be entirely routine (a malformed payload from one device) while
/// a `Transient` one may be alarming (the broker has been unreachable for a
/// day). What the variant decides is whether the operation is retried,
/// quarantined, or the process exits.
///
/// The reference table mapping concrete failures to variants is in
/// [ADR-014](../../../../docs/adr/014-failure-and-retry-policy.md) §Decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureKind {
    /// Retry with the shared full-jitter backoff — the operation may succeed later.
    ///
    /// Examples: MQTT connection refused, `SQLITE_BUSY`, cloud 5xx or timeout.
    Transient,

    /// Never retry. Quarantine the item and surface it to an operator.
    ///
    /// Retrying will fail identically forever, and a permanently-failing item
    /// at the head of a queue blocks everything behind it. Examples: a
    /// malformed MQTT payload, an envelope/topic `device_id` mismatch, a cloud
    /// 4xx that is not 429.
    Permanent,

    /// The process cannot continue correctly — log at ERROR and exit non-zero.
    ///
    /// A process that is up but not evaluating safety is worse than a process
    /// that is down, because supervision and alerting see "healthy" while
    /// nothing is watching the plant. Examples: migration failure at startup,
    /// invalid configuration, a control-loop task panic.
    Fatal,
}

impl FailureKind {
    /// Whether the operation that produced this failure may be retried.
    ///
    /// True only for [`FailureKind::Transient`]. Note that "may be retried"
    /// is a property of the *failure*, not of the operation: an operation
    /// which is not idempotent must not be retried even on a transient
    /// failure. Command publication is the worked example — ADR-014 requires
    /// the same `command_id` to be republished, never a fresh one.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::Transient)
    }

    /// Whether encountering this failure requires the process to exit.
    #[must_use]
    pub const fn is_fatal(self) -> bool {
        matches!(self, Self::Fatal)
    }

    /// A stable lowercase label, suitable as a metric label or a log field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Permanent => "permanent",
            Self::Fatal => "fatal",
        }
    }
}

impl fmt::Display for FailureKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Classifies an error into the single [`FailureKind`] that governs it.
///
/// # The rule that makes this worth having
///
/// **Every implementation matches exhaustively, with no catch-all arm.**
///
/// ```ignore
/// impl Classify for StorageError {
///     fn classify(&self) -> FailureKind {
///         match self {
///             Self::Busy => FailureKind::Transient,
///             Self::DiskFull => FailureKind::Fatal,
///             Self::ConstraintViolation => FailureKind::Permanent,
///             // no `_ =>` arm, deliberately
///         }
///     }
/// }
/// ```
///
/// A `_ => FailureKind::Transient` arm would compile forever, silently
/// swallowing every variant added afterwards into whatever the catch-all
/// happened to say. Without it, a new variant fails to compile until someone
/// decides whether it is retryable — which is the entire point of the trait.
/// That decision is the cheapest possible moment to make it, and the only one
/// where the person adding the variant still has the context.
///
/// Implementations are unit-tested one variant at a time
/// ([ADR-014](../../../../docs/adr/014-failure-and-retry-policy.md)).
pub trait Classify {
    /// The handling class this error falls into.
    fn classify(&self) -> FailureKind;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sample error type showing the intended shape. This is the pattern
    /// every crate's error type follows from M1 onward.
    #[derive(Debug, thiserror::Error)]
    enum SampleError {
        #[error("broker unreachable")]
        BrokerUnreachable,
        #[error("payload is not valid JSON")]
        MalformedPayload,
        #[error("configuration key `{0}` is invalid")]
        InvalidConfig(String),
    }

    impl Classify for SampleError {
        fn classify(&self) -> FailureKind {
            // Exhaustive by construction — no catch-all arm.
            match self {
                Self::BrokerUnreachable => FailureKind::Transient,
                Self::MalformedPayload => FailureKind::Permanent,
                Self::InvalidConfig(_) => FailureKind::Fatal,
            }
        }
    }

    #[test]
    fn classify_transient_variant() {
        let e = SampleError::BrokerUnreachable;
        assert_eq!(e.classify(), FailureKind::Transient);
        assert!(e.classify().is_retryable());
        assert!(!e.classify().is_fatal());
    }

    #[test]
    fn classify_permanent_variant() {
        let e = SampleError::MalformedPayload;
        assert_eq!(e.classify(), FailureKind::Permanent);
        assert!(!e.classify().is_retryable());
        assert!(!e.classify().is_fatal());
    }

    #[test]
    fn classify_fatal_variant() {
        let e = SampleError::InvalidConfig("mqtt.broker_url".into());
        assert_eq!(e.classify(), FailureKind::Fatal);
        assert!(!e.classify().is_retryable());
        assert!(e.classify().is_fatal());
    }

    #[test]
    fn labels_are_stable_and_lowercase() {
        assert_eq!(FailureKind::Transient.as_str(), "transient");
        assert_eq!(FailureKind::Permanent.as_str(), "permanent");
        assert_eq!(FailureKind::Fatal.as_str(), "fatal");
        assert_eq!(FailureKind::Fatal.to_string(), "fatal");
    }
}
