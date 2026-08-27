//! `no_std` wire time representation from ADR-013.
use serde::{Deserialize, Serialize};
/// Unix epoch milliseconds UTC, encoded as a bare JSON integer.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UtcMillis(pub i64);

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    #[test]
    fn bare_integer_round_trip() {
        for value in [-1, 0, 1, 1_756_121_400_000] {
            let t = UtcMillis(value);
            let json = serde_json::to_string(&t).unwrap();
            assert_eq!(json, value.to_string());
            assert_eq!(serde_json::from_str::<UtcMillis>(&json).unwrap(), t);
        }
    }
}
