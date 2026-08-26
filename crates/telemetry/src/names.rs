//! Metric name constants.
//!
//! Every metric name in the system is a `const` here, so a typo at a call site
//! is a compile error rather than a silently missing series that nobody
//! notices until they need it.
//!
//! # The catalogue is deliberately empty in M0
//!
//! [ADR-010](../../../../docs/adr/010-observability-strategy.md) is explicit
//! that metrics are added when a real question cannot be answered without
//! them: a large catalogue nobody reads costs cardinality and maintenance and
//! is worse than a small one that answers the operational questions. M0
//! delivers no behaviour to measure, so it declares no metric.
//!
//! ADR-010 §Metrics carries the normative catalogue and the milestone each
//! entry arrives in — ingestion in M3, control and lockouts in M6, cloud sync
//! in M7. A name added here should match that document.
//!
//! # Naming
//!
//! Prometheus conventions, which are not merely cosmetic — `promtool` and most
//! dashboards assume them:
//!
//! - counters end in `_total`
//! - durations end in `_seconds`, byte counts in `_bytes`
//! - gauges are named for the quantity, not the act of measuring it
//!   (`devices_online`, not `devices_online_count`)
