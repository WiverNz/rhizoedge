//! Assembled-system scenario harness for M8.
//!
//! Every assertion here is on **observable state** — an API response, a row in
//! one of the two databases, or a message the MQTT spy captured at the broker
//! boundary. Never on a log line (F-080-12): a log string is not an interface,
//! and a suite that asserted on one would go red on a reworded message and
//! green on a reworded bug.
#![forbid(unsafe_code)]
// The suite's own unit tests assert against a `static` catalogue whose shape is
// fixed at compile time; a `None` there is a bug in this file, not a runtime
// condition to classify.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod harness;
pub mod scenarios;
