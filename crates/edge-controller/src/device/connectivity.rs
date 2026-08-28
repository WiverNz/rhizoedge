//! Connectivity mode derivation; edge-observed liveness is authoritative.
/// Returns the externally visible mode.
pub fn derive(online: bool, reported_isolated: bool) -> &'static str {
    if !online {
        "reconciling"
    } else if reported_isolated {
        "isolated"
    } else {
        "connected"
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edge_liveness_overrides_advisory_report() {
        assert_eq!(derive(false, false), "reconciling");
        assert_eq!(derive(true, true), "isolated");
        assert_eq!(derive(true, false), "connected");
    }
}
