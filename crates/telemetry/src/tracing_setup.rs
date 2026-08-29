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
use std::io::{self, IsTerminal};
use std::str::FromStr;

use chrono::Local;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber, span};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::field::RecordFields;
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::fmt::{FmtContext, FormattedFields, MakeWriter};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;

use crate::TelemetryError;

/// How log events are rendered.
///
/// JSON in production so events are machine-filterable; compact or pretty in
/// development so they are readable in a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogFormat {
    /// One JSON object per event. The production default.
    #[default]
    Json,
    /// Single-line, correlation-first development output.
    Compact,
    /// Multi-line, human-oriented. Development only.
    Pretty,
}

impl LogFormat {
    /// Every accepted value, for error messages that tell the operator what to
    /// write instead of only what was wrong.
    pub const ACCEPTED: [&'static str; 3] = ["json", "compact", "pretty"];

    /// The canonical lowercase spelling accepted in configuration.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Compact => "compact",
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
            "compact" => Ok(Self::Compact),
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
    init_tracing_with_writer_and_ansi(format, directive, io::stdout, io::stdout().is_terminal())
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
    init_tracing_with_writer_and_ansi(format, filter, writer, false)
}

fn init_tracing_with_writer_and_ansi<W>(
    format: LogFormat,
    filter: &str,
    writer: W,
    ansi: bool,
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
        LogFormat::Compact => registry
            .with(
                tracing_subscriber::fmt::layer()
                    .fmt_fields(CompactFields)
                    .event_format(CompactEvent { ansi })
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

const FIELD_SEPARATOR: char = '\u{1f}';
const CORRELATION_FIELDS: [&str; 9] = [
    "device_id",
    "device",
    "plant_id",
    "command_id",
    "event_id",
    "watering_event_id",
    "message_id",
    "boot_id",
    "batch_id",
];

#[derive(Debug, Clone, Copy)]
struct CompactFields;

impl<'writer> FormatFields<'writer> for CompactFields {
    fn format_fields<R: RecordFields>(&self, writer: Writer<'writer>, fields: R) -> fmt::Result {
        let mut visitor = StoredFieldVisitor {
            writer,
            result: Ok(()),
        };
        fields.record(&mut visitor);
        visitor.result
    }

    fn add_fields(
        &self,
        current: &'writer mut FormattedFields<Self>,
        fields: &span::Record<'_>,
    ) -> fmt::Result {
        self.format_fields(current.as_writer(), fields)
    }
}

struct StoredFieldVisitor<'writer> {
    writer: Writer<'writer>,
    result: fmt::Result,
}

impl StoredFieldVisitor<'_> {
    fn record_value(&mut self, field: &Field, value: impl fmt::Display) {
        if self.result.is_ok() {
            self.result = write!(
                self.writer,
                "{}={}{}",
                field.name(),
                sanitize(&value.to_string()),
                FIELD_SEPARATOR
            );
        }
    }
}

impl Visit for StoredFieldVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record_value(field, format_args!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_value(field, value);
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_value(field, value);
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_value(field, value);
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record_value(field, value);
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.record_value(field, value);
    }
}

#[derive(Debug)]
struct EventFields {
    message: Option<String>,
    fields: Vec<(String, String)>,
}

impl EventFields {
    fn new() -> Self {
        Self {
            message: None,
            fields: Vec::new(),
        }
    }

    fn record_value(&mut self, field: &Field, value: impl fmt::Display) {
        let value = sanitize(&value.to_string());
        if field.name() == "message" {
            self.message = Some(value);
        } else {
            self.fields.push((field.name().to_owned(), value));
        }
    }
}

impl Visit for EventFields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record_value(field, format_args!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_value(field, value);
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_value(field, value);
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_value(field, value);
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record_value(field, value);
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.record_value(field, value);
    }
}

#[derive(Debug, Clone, Copy)]
struct CompactEvent {
    ansi: bool,
}

impl<S> FormatEvent<S, CompactFields> for CompactEvent
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, CompactFields>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();
        let mut event_fields = EventFields::new();
        event.record(&mut event_fields);

        let mut span_fields = Vec::new();
        if let Some(scope) = ctx.event_scope() {
            for span in scope.from_root() {
                if let Some(formatted) = span.extensions().get::<FormattedFields<CompactFields>>() {
                    span_fields.extend(parse_stored_fields(formatted.fields.as_str()));
                }
            }
        }

        write!(writer, "{} ", Local::now().format("%H:%M:%S%.3f"))?;
        self.write_level(&mut writer, metadata.level())?;
        write!(writer, " {:<5}", component(metadata.target()))?;

        let correlation = CORRELATION_FIELDS.iter().find_map(|wanted| {
            event_fields
                .fields
                .iter()
                .chain(span_fields.iter())
                .find(|(name, _)| name == wanted)
                .map(|(_, value)| value.as_str())
        });
        if let Some(value) = correlation {
            write!(writer, " {value}")?;
        }
        if let Some(message) = event_fields.message.as_deref() {
            write!(writer, " {message}")?;
        }

        for (name, value) in span_fields.iter().chain(event_fields.fields.iter()) {
            if correlation.is_some_and(|selected| {
                CORRELATION_FIELDS.contains(&name.as_str()) && selected == value
            }) {
                continue;
            }
            write!(writer, " {name}={value}")?;
        }

        if matches!(
            *metadata.level(),
            Level::ERROR | Level::DEBUG | Level::TRACE
        ) && let Some(file) = metadata.file()
        {
            write!(writer, " source={file}")?;
            if let Some(line) = metadata.line() {
                write!(writer, ":{line}")?;
            }
        }
        writeln!(writer)
    }
}

impl CompactEvent {
    fn write_level(&self, writer: &mut Writer<'_>, level: &Level) -> fmt::Result {
        let color = match *level {
            Level::ERROR => "31",
            Level::WARN => "33",
            Level::INFO => "32",
            Level::DEBUG => "34",
            Level::TRACE => "35",
        };
        if self.ansi {
            write!(writer, "\x1b[{color}m{level:<5}\x1b[0m")
        } else {
            write!(writer, "{level:<5}")
        }
    }
}

fn parse_stored_fields(fields: &str) -> Vec<(String, String)> {
    fields
        .split(FIELD_SEPARATOR)
        .filter(|field| !field.is_empty())
        .filter_map(|field| {
            field
                .split_once('=')
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
        })
        .collect()
}

fn sanitize(value: &str) -> String {
    value
        .replace('\r', "\\r")
        .replace('\n', "\\n")
        .replace(FIELD_SEPARATOR, "\\u001f")
}

fn component(target: &str) -> &'static str {
    if target.starts_with("edge_controller") {
        "EDGE"
    } else if target.starts_with("device_simulator") {
        "SIM"
    } else if target.starts_with("rhizo_storage") {
        "STORE"
    } else if target.starts_with("rhizo_cloud") {
        "CLOUD"
    } else {
        "RHIZO"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_format_parses_canonical_spellings() {
        assert_eq!("json".parse::<LogFormat>().unwrap(), LogFormat::Json);
        assert_eq!("compact".parse::<LogFormat>().unwrap(), LogFormat::Compact);
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
        assert!(msg.contains("compact"), "must list accepted values: {msg}");
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
