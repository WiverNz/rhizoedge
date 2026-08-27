//! Clock abstraction for pure, deterministic domain logic.
use chrono::{DateTime, Utc};
/// Source of authoritative current time.
pub trait Clock: Send + Sync {
    /** Returns the current UTC instant. */
    fn now(&self) -> DateTime<Utc>;
}
/// Production clock adapter. The time-reading function is injected by the
/// binary so this pure crate never calls a wall-clock API directly.
#[derive(Clone, Copy)]
pub struct SystemClock {
    reader: fn() -> DateTime<Utc>,
}
impl SystemClock {
    /** Creates an adapter around a host clock function. */
    pub const fn new(reader: fn() -> DateTime<Utc>) -> Self {
        Self { reader }
    }
}
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        (self.reader)()
    }
}
