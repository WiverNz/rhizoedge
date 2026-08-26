//! Metrics registry and Prometheus text rendering.
//!
//! Pull-based: the edge exposes `/metrics` and a Prometheus scrape or a `curl`
//! in a terminal both work. No push gateway and no agent — a system whose
//! entire premise is working when things are unavailable should not depend on
//! a collector being available
//! ([ADR-010](../../../../docs/adr/010-observability-strategy.md)).
//!
//! # Registration is idempotent
//!
//! The helpers below return the existing metric when one is already registered
//! under that name, so a component that is constructed twice (a reconnect, a
//! test that runs after another test) does not fail on a duplicate
//! registration. The registry is process-wide because the exposition endpoint
//! is.
//!
//! # Cardinality
//!
//! ADR-010's catalogue is normative about labels, and in particular about
//! `device_id`: it appears only on `device_restarts_total`, where the
//! cardinality equals the (small) device count and the per-device breakdown is
//! the point. Adding it to a high-frequency counter multiplies every series by
//! the fleet size. Labelled metrics are registered directly against
//! [`registry`] using the `prometheus` API; the helpers here cover the
//! unlabelled case.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use prometheus::{
    Encoder, Gauge, Histogram, HistogramOpts, IntCounter, Opts, Registry, TextEncoder,
};

use crate::TelemetryError;

/// The process-wide metric registry.
///
/// Every metric that should appear on `/metrics` is registered here.
#[must_use]
pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::new)
}

/// Caches of already-registered metrics, keyed by name, so registration is
/// idempotent. Each `prometheus` metric handle is internally an `Arc`, so
/// cloning one out of the cache is cheap and shares the same counter.
struct Caches {
    counters: Mutex<HashMap<String, IntCounter>>,
    gauges: Mutex<HashMap<String, Gauge>>,
    histograms: Mutex<HashMap<String, Histogram>>,
}

fn caches() -> &'static Caches {
    static CACHES: OnceLock<Caches> = OnceLock::new();
    CACHES.get_or_init(|| Caches {
        counters: Mutex::new(HashMap::new()),
        gauges: Mutex::new(HashMap::new()),
        histograms: Mutex::new(HashMap::new()),
    })
}

/// Recovers a cache lock, discarding poisoning.
///
/// A panic while holding one of these locks can only have happened between a
/// `HashMap` lookup and an insert, which leaves the map itself structurally
/// intact. Refusing to register a metric afterwards would turn an unrelated
/// panic into permanent observability loss, exactly when the panic most needs
/// explaining.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Registers (or returns) a monotonically increasing counter.
///
/// # Errors
///
/// Returns [`TelemetryError::MetricRegistration`] if `name` is not a valid
/// Prometheus metric name, or if a metric of a *different* type is already
/// registered under it.
pub fn counter(name: &str, help: &str) -> Result<IntCounter, TelemetryError> {
    let mut cache = lock(&caches().counters);
    if let Some(existing) = cache.get(name) {
        return Ok(existing.clone());
    }
    let metric =
        IntCounter::with_opts(Opts::new(name, help)).map_err(|source| reg_err(name, &source))?;
    registry()
        .register(Box::new(metric.clone()))
        .map_err(|source| reg_err(name, &source))?;
    cache.insert(name.to_owned(), metric.clone());
    Ok(metric)
}

/// Registers (or returns) a gauge, which may go up or down.
///
/// # Errors
///
/// As [`counter`].
pub fn gauge(name: &str, help: &str) -> Result<Gauge, TelemetryError> {
    let mut cache = lock(&caches().gauges);
    if let Some(existing) = cache.get(name) {
        return Ok(existing.clone());
    }
    let metric =
        Gauge::with_opts(Opts::new(name, help)).map_err(|source| reg_err(name, &source))?;
    registry()
        .register(Box::new(metric.clone()))
        .map_err(|source| reg_err(name, &source))?;
    cache.insert(name.to_owned(), metric.clone());
    Ok(metric)
}

/// Registers (or returns) a histogram with explicit bucket upper bounds.
///
/// Buckets are given explicitly rather than defaulted: the useful boundaries
/// for MQTT processing latency and for a cloud sync batch are three orders of
/// magnitude apart, and a shared default would be wrong for both.
///
/// # Errors
///
/// As [`counter`], and additionally if `buckets` is empty or not sorted
/// ascending.
pub fn histogram(name: &str, help: &str, buckets: &[f64]) -> Result<Histogram, TelemetryError> {
    let mut cache = lock(&caches().histograms);
    if let Some(existing) = cache.get(name) {
        return Ok(existing.clone());
    }
    let opts = HistogramOpts::new(name, help).buckets(buckets.to_vec());
    let metric = Histogram::with_opts(opts).map_err(|source| reg_err(name, &source))?;
    registry()
        .register(Box::new(metric.clone()))
        .map_err(|source| reg_err(name, &source))?;
    cache.insert(name.to_owned(), metric.clone());
    Ok(metric)
}

fn reg_err(name: &str, source: &prometheus::Error) -> TelemetryError {
    TelemetryError::MetricRegistration {
        name: name.to_owned(),
        detail: source.to_string(),
    }
}

/// Renders every registered metric in the Prometheus text exposition format.
///
/// This is the body of `GET /metrics`. Rendering never fails: a metric that
/// cannot be encoded is a bug in registration, and returning an error from the
/// scrape endpoint would lose every *other* series along with it.
#[must_use]
pub fn render_prometheus() -> String {
    let families = registry().gather();
    let mut buf = Vec::new();
    if TextEncoder::new().encode(&families, &mut buf).is_err() {
        return String::new();
    }
    String::from_utf8(buf).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deliberately strict, hand-written check of the exposition grammar:
    /// every line is a `# HELP <name> <text>`, a `# TYPE <name> <type>`, or a
    /// sample `<name>[{labels}] <value>`.
    fn assert_valid_exposition(text: &str) {
        for line in text.lines() {
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("# HELP ") {
                assert!(
                    rest.split_whitespace().next().is_some(),
                    "HELP line has no metric name: {line}"
                );
                continue;
            }
            if let Some(rest) = line.strip_prefix("# TYPE ") {
                let mut parts = rest.split_whitespace();
                let name = parts.next().unwrap_or_default();
                let kind = parts.next().unwrap_or_default();
                assert!(!name.is_empty(), "TYPE line has no metric name: {line}");
                assert!(
                    ["counter", "gauge", "histogram", "summary", "untyped"].contains(&kind),
                    "unknown metric type {kind:?} in: {line}"
                );
                continue;
            }
            assert!(!line.starts_with('#'), "unrecognised comment line: {line}");

            let (name_and_labels, value) = line
                .rsplit_once(' ')
                .unwrap_or_else(|| panic!("sample line has no value: {line}"));
            let name = name_and_labels
                .split_once('{')
                .map_or(name_and_labels, |(n, _)| n);
            assert!(
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':'),
                "invalid metric name {name:?} in: {line}"
            );
            assert!(
                value.parse::<f64>().is_ok()
                    || value == "NaN"
                    || value == "+Inf"
                    || value == "-Inf",
                "invalid sample value {value:?} in: {line}"
            );
        }
    }

    #[test]
    fn counter_renders_as_a_counter() {
        let c = counter("test_counter_total", "a test counter").unwrap();
        c.inc();
        c.inc_by(4);
        let text = render_prometheus();
        assert!(text.contains("# TYPE test_counter_total counter"), "{text}");
        assert!(text.contains("test_counter_total 5"), "{text}");
        assert_valid_exposition(&text);
    }

    #[test]
    fn gauge_renders_as_a_gauge_and_can_decrease() {
        let g = gauge("test_gauge", "a test gauge").unwrap();
        g.set(7.0);
        g.dec();
        let text = render_prometheus();
        assert!(text.contains("# TYPE test_gauge gauge"), "{text}");
        assert!(text.contains("test_gauge 6"), "{text}");
        assert_valid_exposition(&text);
    }

    #[test]
    fn histogram_renders_buckets_sum_and_count() {
        let h = histogram(
            "test_histogram_seconds",
            "a test histogram",
            &[0.1, 1.0, 10.0],
        )
        .unwrap();
        h.observe(0.05);
        h.observe(2.0);
        let text = render_prometheus();
        assert!(
            text.contains("# TYPE test_histogram_seconds histogram"),
            "{text}"
        );
        assert!(
            text.contains("test_histogram_seconds_bucket{le=\"0.1\"} 1"),
            "{text}"
        );
        assert!(text.contains("test_histogram_seconds_count 2"), "{text}");
        assert!(text.contains("test_histogram_seconds_sum"), "{text}");
        assert_valid_exposition(&text);
    }

    #[test]
    fn registration_is_idempotent_and_shares_state() {
        let a = counter("test_idempotent_total", "registered twice").unwrap();
        a.inc();
        let b = counter("test_idempotent_total", "registered twice").unwrap();
        b.inc();
        // Same underlying metric, so the second handle sees the first's value.
        assert_eq!(b.get(), 2);
        let text = render_prometheus();
        assert_eq!(
            text.matches("# TYPE test_idempotent_total counter").count(),
            1,
            "the series must appear exactly once: {text}"
        );
    }

    #[test]
    fn an_invalid_metric_name_is_rejected_by_name() {
        let err = counter("not a valid name", "help").unwrap_err();
        assert!(
            err.to_string().contains("not a valid name"),
            "error must name the offending metric: {err}"
        );
    }

    #[test]
    fn an_empty_registry_renders_empty_but_valid() {
        // Not the global registry — this asserts the format of the empty case
        // without depending on which other tests have run.
        let empty = Registry::new();
        let mut buf = Vec::new();
        TextEncoder::new()
            .encode(&empty.gather(), &mut buf)
            .unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.is_empty());
        assert_valid_exposition(&text);
    }
}
