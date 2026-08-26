//! The crate's error type.

use crate::{Classify, FailureKind};

/// Anything that can go wrong while wiring observability.
///
/// Every variant is a startup or registration problem. There is deliberately
/// no variant for "failed to emit a log line" or "failed to increment a
/// counter": observability must never be able to fail an operation it is only
/// there to describe.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TelemetryError {
    /// The configured log format is not one this build understands.
    #[error("unknown log format `{value}` (accepted: {accepted})")]
    UnknownLogFormat {
        /// The value that was supplied.
        value: String,
        /// The comma-separated set of accepted values.
        accepted: String,
    },

    /// The `RUST_LOG`-style filter directive could not be parsed.
    #[error("invalid log filter directive `{directive}`: {detail}")]
    InvalidLogFilter {
        /// The directive that was supplied.
        directive: String,
        /// The parser's own message, which names the offending fragment.
        detail: String,
    },

    /// A global tracing subscriber was already installed.
    #[error("tracing subscriber already initialised: {detail}")]
    AlreadyInitialised {
        /// The underlying message from `tracing-subscriber`.
        detail: String,
    },

    /// A metric could not be registered.
    #[error("cannot register metric `{name}`: {detail}")]
    MetricRegistration {
        /// The metric name that was rejected.
        name: String,
        /// The registry's own message.
        detail: String,
    },
}

impl Classify for TelemetryError {
    /// Every telemetry failure is fatal.
    ///
    /// Exhaustive with no catch-all arm: if a future variant is genuinely
    /// recoverable, this stops compiling until someone says so explicitly.
    fn classify(&self) -> FailureKind {
        match self {
            // A misconfigured log format or filter is invalid configuration,
            // and ADR-014 classifies invalid configuration as Fatal. Starting
            // with logging silently degraded is exactly the "started with a
            // default substituted for something the operator got wrong" case
            // that ADR-011 exists to prevent.
            Self::UnknownLogFormat { .. } | Self::InvalidLogFilter { .. } => FailureKind::Fatal,
            // Double initialisation and a bad metric name are both programming
            // errors, not conditions that a retry could clear.
            Self::AlreadyInitialised { .. } | Self::MetricRegistration { .. } => FailureKind::Fatal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_is_fatal() {
        let cases = [
            TelemetryError::UnknownLogFormat {
                value: "yaml".into(),
                accepted: "json, pretty".into(),
            },
            TelemetryError::InvalidLogFilter {
                directive: "==".into(),
                detail: "bad".into(),
            },
            TelemetryError::AlreadyInitialised {
                detail: "already set".into(),
            },
            TelemetryError::MetricRegistration {
                name: "bad name".into(),
                detail: "invalid".into(),
            },
        ];
        for case in cases {
            assert_eq!(case.classify(), FailureKind::Fatal, "{case}");
            assert!(!case.classify().is_retryable());
        }
    }
}
