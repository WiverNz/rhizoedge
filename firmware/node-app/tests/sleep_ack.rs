//! Test the production sleep handshake's actual batch classifier.
#![allow(clippy::unwrap_used)]
#[path = "../../esp32-node/src/net/inbound.rs"]
mod inbound;
#[path = "../../esp32-node/src/net/sleep_ack.rs"]
mod sleep_ack;
use inbound::Inbound;

#[test]
fn unrelated_puback_cannot_authorize_sleep() {
    let result = sleep_ack::classify(42, vec![Inbound::PublishAcknowledged(41)]);
    assert!(!result.acknowledged);
}
#[test]
fn matching_ack_preserves_pending_commands_and_disconnect_in_order() {
    let result = sleep_ack::classify(
        42,
        vec![
            Inbound::Connected,
            Inbound::Message {
                topic: None,
                payload: vec![7, 8],
            },
            Inbound::PublishAcknowledged(42),
            Inbound::Disconnected,
        ],
    );
    assert!(result.acknowledged);
    assert_eq!(result.pending.len(), 3);
    assert!(matches!(result.pending[0], Inbound::Connected));
    assert!(
        matches!(&result.pending[1], Inbound::Message { topic: None, payload } if payload == &[7, 8])
    );
    assert!(matches!(result.pending[2], Inbound::Disconnected));
}
#[test]
fn empty_or_duplicate_other_acks_do_not_authorize_sleep() {
    assert!(!sleep_ack::classify(42, vec![]).acknowledged);
    assert!(
        !sleep_ack::classify(
            42,
            vec![
                Inbound::PublishAcknowledged(1),
                Inbound::PublishAcknowledged(1)
            ]
        )
        .acknowledged
    );
    assert!(sleep_ack::classify(42, vec![Inbound::PublishAcknowledged(42)]).acknowledged);
}

#[test]
fn sleep_runtime_uses_publish_id_and_preserves_deferred_dispatch() {
    let run = include_str!("../../esp32-node/src/run.rs");
    let sleep = run.split("fn announce_and_sleep").nth(1).unwrap();
    assert!(sleep.contains("let Some(packet_id) = publish_with_id"));
    assert!(sleep.contains("sleep_ack::classify(packet_id, session.drain())"));
    assert!(sleep.contains("session.defer(batch.pending)"));
    assert!(
        sleep.find("if !acknowledged").unwrap()
            < sleep.find("cycle.announcement_acknowledged()").unwrap()
    );
    let session = include_str!("../../esp32-node/src/net/session.rs");
    assert!(session.contains("core::mem::take(&mut self.pending)"));
}
