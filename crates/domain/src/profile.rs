//! Minimal M1 profile template and sample primitives.
use crate::ProfileId;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
/// Template used only to seed plant-owned policies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlantProfile {
    /** Template id. */
    pub profile_id: ProfileId,
    /** Human name. */
    pub name: String,
    /** Suggested lower moisture target. */
    pub target_min_vwc: f64,
    /** Suggested upper moisture target. */
    pub target_max_vwc: f64,
}
impl PlantProfile {
    /** Rejects incoherent targets. */
    pub fn is_valid(&self) -> bool {
        self.target_min_vwc.is_finite()
            && self.target_max_vwc.is_finite()
            && self.target_min_vwc >= 0.0
            && self.target_min_vwc < self.target_max_vwc
            && self.target_max_vwc <= 100.0
    }
}
/// Backward-compatible soil-domain view used by early recommendation work.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoilSample {
    /** Moisture percentage. */
    pub moisture_vwc: Option<f64>,
    /** Edge authoritative receipt time. */
    pub received_at: DateTime<Utc>,
}
impl SoilSample {
    /** Physical plausibility only. */
    pub fn is_valid(&self) -> bool {
        self.moisture_vwc
            .is_some_and(|v| v.is_finite() && (0.0..=100.0).contains(&v))
    }
    /** Uses edge receipt time and is stale at the exact boundary. */
    pub fn is_stale(&self, now: DateTime<Utc>, max_age: Duration) -> bool {
        now.signed_duration_since(self.received_at) >= max_age
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    #[test]
    fn validity_and_staleness_boundaries() {
        let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let s = SoilSample {
            moisture_vwc: Some(0.),
            received_at: at,
        };
        assert!(s.is_valid());
        assert!(!s.is_stale(at + Duration::seconds(9), Duration::seconds(10)));
        assert!(s.is_stale(at + Duration::seconds(10), Duration::seconds(10)));
    }
}
