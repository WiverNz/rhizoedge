//! A string that does not print itself.

use std::fmt;

use serde::Deserialize;

/// A secret value whose `Debug` and `Display` render `[redacted]`.
///
/// Every secret-shaped field in the configuration is this type, so redaction
/// is a property of the *type* rather than of each formatting call site
/// ([ADR-010](../../../../docs/adr/010-observability-strategy.md) §Redaction).
/// A field-name-matching redactor would silently stop working the moment
/// someone renamed `password` to `credential`; a type cannot be leaked by
/// being renamed.
///
/// Reading the value requires [`expose`](Self::expose), which is deliberately
/// ugly to type and easy to grep for.
///
/// ```
/// # use edge_controller::config::Secret;
/// let s = Secret::from("hunter2");
/// assert_eq!(format!("{s:?}"), "[redacted]");
/// assert_eq!(s.to_string(), "[redacted]");
/// assert_eq!(s.expose(), "hunter2");
/// ```
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    /// The placeholder every rendering of a secret produces.
    pub const REDACTED: &'static str = "[redacted]";

    /// Returns the underlying value.
    ///
    /// Every call site is a place a secret can escape. Keep them few, and keep
    /// them next to the thing that genuinely needs the bytes — an MQTT
    /// connection, not a log line.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether the secret is unset.
    ///
    /// Distinguishing "no password configured" from "password configured"
    /// without revealing which one it is.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<&str> for Secret {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for Secret {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(Self::REDACTED)
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(Self::REDACTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_both_redact() {
        let s = Secret::from("s3cr3t-value");
        assert_eq!(format!("{s:?}"), Secret::REDACTED);
        assert_eq!(format!("{s}"), Secret::REDACTED);
        assert!(!format!("{s:?} {s}").contains("s3cr3t-value"));
    }

    #[test]
    fn an_empty_secret_redacts_too() {
        // Rendering an empty secret as `""` would leak the fact that no
        // password is set, which is exactly the thing an attacker probes for.
        let s = Secret::default();
        assert_eq!(format!("{s:?}"), Secret::REDACTED);
        assert!(s.is_empty());
    }

    #[test]
    fn expose_returns_the_real_value() {
        assert_eq!(Secret::from("hunter2").expose(), "hunter2");
    }

    #[test]
    fn a_secret_nested_in_a_derived_debug_is_still_redacted() {
        // The fields exist to be *formatted*, which dead-code analysis does
        // not count as a read.
        #[derive(Debug)]
        #[allow(dead_code, reason = "read only through the derived Debug impl")]
        struct Holder {
            username: String,
            password: Secret,
        }
        let h = Holder {
            username: "rhizo-edge".into(),
            password: Secret::from("do-not-print-me"),
        };
        let rendered = format!("{h:?}");
        assert!(rendered.contains("rhizo-edge"));
        assert!(rendered.contains(Secret::REDACTED));
        assert!(!rendered.contains("do-not-print-me"));
    }
}
