//! Structured logging setup.
//!
//! `tracing` with JSON in production and a human-readable format in
//! development, selected by `RHIZO_EDGE__LOG__FORMAT`
//! ([ADR-010](../../../../docs/adr/010-observability-strategy.md)).
//!
//! # Structured fields, not interpolated strings
//!
//! Correlation identifiers are recorded as *fields*. A `device_id` formatted
//! into the message text is not searchable, which defeats the entire reason
//! this crate exists: the operational question is always "what did the system
//! think was happening, and when?", and answering it means filtering by
//! `plant_id` or `command_id` across ingestion, control, and sync.
//!
//! ```text
//! // wrong — unsearchable
//! info!("watering plant {} with {} ml", plant_id, ml);
//!
//! // right
//! info!(plant_id = %plant_id, requested_ml = ml, mode = %mode, "issuing dose");
//! ```
//!
//! Spans mark units of work, so every event inside one inherits its
//! correlation fields without repeating them at each call site.

use std::fmt;
use std::io;
use std::str::FromStr;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::TelemetryError;

/// How log events are rendered.
///
/// JSON in production so events are machine-filterable; pretty in development
/// so they are readable in a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// One JSON object per event. The production default.
    #[default]
    Json,
    /// Multi-line, human-oriented. Development only.
    Pretty,
}

impl LogFormat {
    /// Every accepted value, for error messages that tell the operator what to
    /// write instead of only what was wrong.
    pub const ACCEPTED: [&'static str; 2] = ["json", "pretty"];

    /// The canonical lowercase spelling accepted in configuration.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Pretty => "pretty",
        }
    }
}

impl fmt::Display for LogFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LogFormat {
    type Err = TelemetryError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "pretty" => Ok(Self::Pretty),
            other => Err(TelemetryError::UnknownLogFormat {
                value: other.to_owned(),
                accepted: Self::ACCEPTED.join(", "),
            }),
        }
    }
}

/// The `RUST_LOG` environment variable, read once at initialisation.
const ENV_FILTER_VAR: &str = "RUST_LOG";

/// Chooses between an environment override and the configured directive.
///
/// `RUST_LOG` wins when it is set to something non-empty, so raising verbosity
/// on a running deployment is a restart with one variable rather than a
/// configuration change. A variable that is present but empty is treated as
/// absent — an empty `RUST_LOG` is how a shell script unsets one, and honouring
/// it literally would silence the process.
///
/// Pure and taking the environment as an argument, so the precedence rule can
/// be tested without mutating the process environment.
fn resolve_directive<'a>(configured: &'a str, env_override: Option<&'a str>) -> &'a str {
    match env_override {
        Some(v) if !v.trim().is_empty() => v,
        _ => configured,
    }
}

/// Builds an [`EnvFilter`] from a directive, naming the directive on failure.
/// Checks that a directive is a valid filter without installing anything.
///
/// Configuration validation needs this: an edge whose `log.level` is a typo
/// should refuse to start naming that key, rather than discovering the problem
/// at the moment it first tries to log something. Having it here rather than
/// in the config crate keeps one definition of the directive grammar.
///
/// # Errors
///
/// Returns [`TelemetryError::InvalidLogFilter`], quoting the directive.
pub fn validate_filter(directive: &str) -> Result<(), TelemetryError> {
    build_filter(directive).map(|_| ())
}

fn build_filter(directive: &str) -> Result<EnvFilter, TelemetryError> {
    EnvFilter::try_new(directive).map_err(|e| TelemetryError::InvalidLogFilter {
        directive: directive.to_owned(),
        detail: e.to_string(),
    })
}

/// Installs the process-wide tracing subscriber, writing to stdout.
///
/// `filter` is a `RUST_LOG`-compatible directive such as `info` or
/// `info,rhizo_storage=debug`. When the `RUST_LOG` environment variable is set
/// and non-empty it takes precedence.
///
/// The subscriber is composed as a layered `Registry`, so an OpenTelemetry
/// layer can be added later without touching a single call site — which is why
/// ADR-010 can defer distributed tracing without painting the project into a
/// corner.
///
/// # Errors
///
/// Returns [`TelemetryError::InvalidLogFilter`] if the effective directive is
/// not valid, and [`TelemetryError::AlreadyInitialised`] if a global
/// subscriber has already been installed.
pub fn init_tracing(format: LogFormat, filter: &str) -> Result<(), TelemetryError> {
    let from_env = std::env::var(ENV_FILTER_VAR).ok();
    let directive = resolve_directive(filter, from_env.as_deref());
    init_tracing_with_writer(format, directive, io::stdout)
}

/// [`init_tracing`] against an explicit writer and an explicit directive.
///
/// `filter` is used verbatim: unlike [`init_tracing`], this function does not
/// consult `RUST_LOG`. That keeps it deterministic, which is the point — tests
/// assert on the exact bytes a real subscriber produces rather than on a
/// reimplementation of its formatting, and must not depend on the developer's
/// environment.
///
/// # Errors
///
/// As [`init_tracing`].
pub fn init_tracing_with_writer<W>(
    format: LogFormat,
    filter: &str,
    writer: W,
) -> Result<(), TelemetryError>
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let env_filter = build_filter(filter)?;
    let registry = tracing_subscriber::registry().with(env_filter);

    let result = match format {
        LogFormat::Json => registry
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_current_span(true)
                    .with_span_list(false)
                    .with_target(true)
                    .with_writer(writer),
            )
            .try_init(),
        LogFormat::Pretty => registry
            .with(
                tracing_subscriber::fmt::layer()
                    .pretty()
                    .with_ansi(false)
                    .with_target(true)
                    .with_writer(writer),
            )
            .try_init(),
    };

    result.map_err(|e| TelemetryError::AlreadyInitialised {
        detail: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_format_parses_canonical_spellings() {
        assert_eq!("json".parse::<LogFormat>().unwrap(), LogFormat::Json);
        assert_eq!("pretty".parse::<LogFormat>().unwrap(), LogFormat::Pretty);
    }

    #[test]
    fn log_format_parsing_is_case_and_whitespace_insensitive() {
        assert_eq!("  JSON ".parse::<LogFormat>().unwrap(), LogFormat::Json);
        assert_eq!("Pretty".parse::<LogFormat>().unwrap(), LogFormat::Pretty);
    }

    #[test]
    fn unknown_log_format_names_the_value_and_the_accepted_set() {
        let err = "yaml".parse::<LogFormat>().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("yaml"), "must name the bad value: {msg}");
        assert!(msg.contains("json"), "must list accepted values: {msg}");
        assert!(msg.contains("pretty"), "must list accepted values: {msg}");
    }

    #[test]
    fn default_log_format_is_json() {
        assert_eq!(LogFormat::default(), LogFormat::Json);
    }

    #[test]
    fn rust_log_overrides_the_configured_directive() {
        assert_eq!(resolve_directive("info", Some("debug")), "debug");
        assert_eq!(
            resolve_directive("info", Some("info,rhizo_storage=trace")),
            "info,rhizo_storage=trace"
        );
    }

    #[test]
    fn an_absent_or_empty_rust_log_leaves_the_configured_directive_in_place() {
        assert_eq!(resolve_directive("info", None), "info");
        assert_eq!(resolve_directive("info", Some("")), "info");
        assert_eq!(resolve_directive("info", Some("   ")), "info");
    }

    #[test]
    fn invalid_filter_directive_is_rejected_and_names_the_directive() {
        let err = build_filter("info=info=info").unwrap_err();
        assert!(matches!(err, TelemetryError::InvalidLogFilter { .. }));
        assert!(
            err.to_string().contains("info=info=info"),
            "must quote the directive: {err}"
        );
    }

    #[test]
    fn valid_filter_directives_are_accepted() {
        assert!(build_filter("info").is_ok());
        assert!(build_filter("info,rhizo_storage=debug").is_ok());
        assert!(build_filter("trace").is_ok());
    }
}
