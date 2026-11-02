//! Exercise the actual production diagnostic representation.
#[path = "../../esp32-node/src/net/diagnostic.rs"]
mod diagnostic;

#[test]
fn reason_and_boot_time_are_preserved_without_narrowing() {
    let record = diagnostic::Record {
        at_ms: u64::MAX,
        attempt: u32::MAX,
        event: "sta_disconnected",
        reason: Some(u16::MAX),
    };
    assert_eq!(
        record.to_string(),
        "NET_DIAG at_ms=18446744073709551615 attempt=4294967295 event=sta_disconnected reason=65535"
    );
}

#[test]
fn missing_reason_is_not_fabricated_as_success_or_zero() {
    let record = diagnostic::Record {
        at_ms: 0,
        attempt: 0,
        event: "sta_start",
        reason: None,
    };
    assert_eq!(
        record.to_string(),
        "NET_DIAG at_ms=0 attempt=0 event=sta_start"
    );
}
