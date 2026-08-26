//! Configuration failures.

use rhizo_telemetry::{Classify, FailureKind};

/// Why the configuration could not be loaded or is not usable.
///
/// Every variant names the offending key. That is the whole design goal: an
/// operator reading a startup failure should be able to go straight to the
/// line they got wrong, without bisecting their `edge.toml`
/// ([ADR-011](../../../../docs/adr/011-configuration-and-secrets-model.md)).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// The named configuration file was requested explicitly but is not there.
    ///
    /// A *missing* file is only an error when the operator asked for it by
    /// path. With no `--config`, the absence of `edge.toml` is normal: PRD 000
    /// requires defaults plus environment to be a working configuration.
    #[error("configuration file `{path}` was requested but does not exist")]
    FileNotFound {
        /// The path as the operator wrote it.
        path: String,
    },

    /// A layer could not be parsed, or a value had the wrong type.
    ///
    /// `key` is the dotted path (`control.tick_interval_seconds`), and
    /// `source` names the layer it came from, because the same key can arrive
    /// from a file, an environment variable, or a flag, and knowing which one
    /// is usually the whole answer.
    #[error("invalid configuration at `{key}` (from {layer}): {detail}")]
    Malformed {
        /// The dotted key path.
        key: String,
        /// The layer that supplied the value.
        layer: String,
        /// The underlying parser's message.
        detail: String,
    },

    /// A value parsed correctly but is not acceptable.
    #[error("invalid value for `{key}`: {detail}")]
    Invalid {
        /// The dotted key path.
        key: String,
        /// What is wrong, and what would be right.
        detail: String,
    },
}

impl ConfigError {
    /// Builds an [`Invalid`](Self::Invalid) error.
    pub(crate) fn invalid(key: &str, detail: impl Into<String>) -> Self {
        Self::Invalid {
            key: key.to_owned(),
            detail: detail.into(),
        }
    }

    /// The configuration key this error is about.
    ///
    /// Available separately from the message so a caller can act on it — a
    /// test asserts on it, and a future API surface could point at the field.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::FileNotFound { path } => path,
            Self::Malformed { key, .. } | Self::Invalid { key, .. } => key,
        }
    }
}

impl Classify for ConfigError {
    /// Invalid configuration is always Fatal.
    ///
    /// ADR-014 classifies it so, and ADR-011 explains why: an edge that starts
    /// with a default silently substituted for something the operator got
    /// wrong is worse than one that refuses to start. Retrying cannot make a
    /// typo correct, and there is no partial configuration worth running with.
    ///
    /// Exhaustive with no catch-all arm, so a future variant has to be
    /// classified deliberately.
    fn classify(&self) -> FailureKind {
        match self {
            Self::FileNotFound { .. } | Self::Malformed { .. } | Self::Invalid { .. } => {
                FailureKind::Fatal
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_is_fatal() {
        let cases = [
            ConfigError::FileNotFound {
                path: "/nope.toml".into(),
            },
            ConfigError::Malformed {
                key: "control.tick_interval_seconds".into(),
                layer: "TOML file".into(),
                detail: "expected u64".into(),
            },
            ConfigError::invalid("api.bind", "not a socket address"),
        ];
        for case in cases {
            assert_eq!(case.classify(), FailureKind::Fatal, "{case}");
        }
    }

    #[test]
    fn the_key_is_available_without_parsing_the_message() {
        let e = ConfigError::invalid("mqtt.broker_url", "unsupported scheme");
        assert_eq!(e.key(), "mqtt.broker_url");
        assert!(e.to_string().contains("mqtt.broker_url"));
    }
}
