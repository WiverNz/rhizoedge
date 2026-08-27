//! Buffering while isolated, and replaying idempotently on reconnection
//! (SCEN-100, SCEN-101, SCEN-104; SAFETY-016, SAFETY-020).
//!
//! Tests inject typed events directly, because real autonomous outcomes are not
//! produced until M6-019. What is under test is the **mechanism** — stable
//! identity, ordering, tiered retention, gap reporting, and idempotent replay —
//! and that mechanism is complete now.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Parser;
use device_simulator::buffer::{AUDIT_CAPACITY, REPLAY_BATCH, TELEMETRY_CAPACITY};
use device_simulator::envelope::Publication;
use device_simulator::{Cli, Device};
use rhizo_mqtt_contract::payload::{DeviceEventBatch, EventDetail, EventKind, EventTier};
use rhizo_mqtt_contract::{Envelope, EventId, Topic};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch_state_file() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push("rhizo-replay-tests");
    let _ = std::fs::create_dir_all(&path);
    path.push(format!(
        "{}-{}.state.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    for extension in ["json", "json.corrupt", "json.tmp"] {
        let _ = std::fs::remove_file(path.with_extension(extension));
    }
    let _ = std::fs::remove_file(&path);
    path
}

fn settings_at(state_file: &str) -> Cli {
    let cli = Cli::try_parse_from([
        "device-simulator",
        "--device-id",
        "plant-node-01",
        "--telemetry-interval",
        "10",
        "--state-file",
        state_file,
    ])
    .expect("test flags must parse");
    cli.validate().expect("test flags must validate");
    cli
}

fn batches(published: &[Publication]) -> Vec<DeviceEventBatch> {
    published
        .iter()
        .filter(|p| matches!(p.topic, Topic::Events(_)))
        .map(|p| {
            let decoded = Envelope::<DeviceEventBatch>::from_json(p.payload.as_bytes()).unwrap();
            decoded.check_topic(&p.topic).unwrap();
            decoded.data
        })
        .collect()
}

fn replayed_ids(published: &[Publication]) -> Vec<EventId> {
    batches(published)
        .into_iter()
        .flat_map(|b| b.events)
        .map(|e| e.event_id)
        .collect()
}

/// Injects a typed audit event, as M6-019's autonomous outcomes will.
fn inject_audit(device: &mut Device, reason: &str) {
    device.record_event(
        EventTier::Audit,
        EventKind::OfflineRefused,
        EventDetail::Refused {
            reason: reason.to_owned(),
        },
    );
}

fn inject_telemetry(device: &mut Device) {
    device.record_event(
        EventTier::Telemetry,
        EventKind::Unknown(String::from("telemetry.sample")),
        EventDetail::Unknown,
    );
}

// ------------------------------------------------------------- SCEN-100

#[test]
fn events_buffer_while_isolated_and_replay_on_reconnect() {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file));
    // Drain the empty replay the first connection sends.
    device.on_connected().unwrap();
    device.on_disconnected();

    for n in 0..5 {
        inject_audit(&mut device, &format!("refusal-{n}"));
    }
    assert_eq!(device.buffered_events(), 5);

    let published = device.on_connected().unwrap();
    let batches = batches(&published);
    assert!(!batches.is_empty(), "reconnecting replays history");
    let events: Vec<_> = batches.iter().flat_map(|b| b.events.clone()).collect();
    assert_eq!(events.len(), 5);
    assert!(batches.iter().all(|b| b.replay));
    assert!(
        batches.last().unwrap().complete,
        "the final batch reports complete"
    );

    let sequences: Vec<_> = events.iter().map(|e| e.device_seq).collect();
    let mut sorted = sequences.clone();
    sorted.sort_unstable();
    assert_eq!(sequences, sorted, "oldest first");
}

#[test]
fn a_device_with_nothing_buffered_still_sends_one_complete_batch() {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file));
    let batches = batches(&device.on_connected().unwrap());
    assert_eq!(batches.len(), 1);
    assert!(batches[0].events.is_empty());
    assert!(
        batches[0].complete,
        "the edge holds every plant in Uncertain until it sees this flag; a \
         device with nothing to say still has to say so"
    );
}

// ------------------------------------------------------------- SCEN-101

/// SAFETY-016: replay is idempotent on `event_id`.
#[test]
fn safety_016_replay_is_idempotent() {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file));
    device.on_connected().unwrap();
    device.on_disconnected();
    for n in 0..10 {
        inject_audit(&mut device, &format!("refusal-{n}"));
    }

    let mut counts: HashMap<EventId, usize> = HashMap::new();
    let mut first: Option<Vec<EventId>> = None;
    for round in 0..3 {
        // Disconnect and reconnect, as an edge crashing mid-reconciliation
        // would cause. No acknowledgement, so everything replays again.
        device.on_disconnected();
        let published = device.on_connected().unwrap();
        let ids = replayed_ids(&published);
        assert_eq!(ids.len(), 10, "round {round} must replay everything");
        for id in &ids {
            *counts.entry(*id).or_default() += 1;
        }
        match &first {
            None => first = Some(ids),
            Some(previous) => assert_eq!(
                &ids, previous,
                "round {round}: ids must be byte-identical across replays"
            ),
        }
    }
    assert_eq!(counts.len(), 10, "ten distinct events, not thirty");
    assert!(counts.values().all(|c| *c == 3));
}

#[test]
fn event_ids_survive_a_restart_unchanged() {
    let state_file = scratch_state_file().display().to_string();
    let before;
    {
        let mut device = Device::new(&settings_at(&state_file));
        device.on_connected().unwrap();
        device.on_disconnected();
        for n in 0..5 {
            inject_audit(&mut device, &format!("refusal-{n}"));
        }
        before = replayed_ids(&device.on_connected().unwrap());
    }
    let mut device = Device::new(&settings_at(&state_file));
    let after = replayed_ids(&device.on_connected().unwrap());
    assert_eq!(
        after, before,
        "a reboot must not regenerate identity; the edge would record the same \
         history twice"
    );
}

#[test]
fn unacknowledged_events_are_replayed_again_and_acknowledged_ones_are_not() {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file));
    device.on_connected().unwrap();
    device.on_disconnected();
    for n in 0..6 {
        inject_audit(&mut device, &format!("refusal-{n}"));
    }

    let published = device.on_connected().unwrap();
    let events: Vec<_> = batches(&published)
        .into_iter()
        .flat_map(|b| b.events)
        .collect();
    assert_eq!(events.len(), 6);

    // The edge acknowledges the first three.
    device.acknowledge_events(events[2].device_seq);
    device.on_disconnected();
    let again = replayed_ids(&device.on_connected().unwrap());
    assert_eq!(again.len(), 3, "only the unacknowledged remain");
    assert_eq!(again[0], events[3].event_id);
}

// ------------------------------------------------------------- SCEN-104

/// SAFETY-020: overflow is reported as an explicit gap, and audit survives.
#[test]
fn safety_020_overflow_emits_gap_marker() {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file));
    device.on_connected().unwrap();
    device.on_disconnected();

    for n in 0..10 {
        inject_audit(&mut device, &format!("dose-{n}"));
    }
    let audit_before = device
        .store()
        .state()
        .offline_events
        .replay_events()
        .into_iter()
        .filter(|e| e.tier == EventTier::Audit)
        .map(|e| e.event_id)
        .collect::<Vec<_>>();

    // Far more telemetry than the tier can hold.
    let overflow = 40;
    for _ in 0..(TELEMETRY_CAPACITY + overflow) {
        inject_telemetry(&mut device);
    }

    let published = device.on_connected().unwrap();
    let events: Vec<_> = batches(&published)
        .into_iter()
        .flat_map(|b| b.events)
        .collect();

    let audit_after: Vec<_> = events
        .iter()
        .filter(|e| e.tier == EventTier::Audit && e.kind != EventKind::HistoryGap)
        .map(|e| e.event_id)
        .collect();
    assert_eq!(
        audit_after, audit_before,
        "audit events are never evicted to make room for telemetry"
    );

    let gap = events
        .iter()
        .find(|e| e.kind == EventKind::HistoryGap)
        .expect("a lost event must be reported, never silently absorbed");
    let EventDetail::Gap {
        lost_count,
        lost_tier,
        from_seq,
        to_seq,
    } = &gap.detail
    else {
        panic!("a history.gap must carry gap detail");
    };
    assert_eq!(*lost_count as usize, overflow);
    assert_eq!(*lost_tier, EventTier::Telemetry);
    assert!(to_seq >= from_seq);
}

#[test]
fn an_audit_storm_evicts_audit_and_says_so() {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file));
    device.on_connected().unwrap();
    device.on_disconnected();

    for n in 0..(AUDIT_CAPACITY + 5) {
        inject_audit(&mut device, &format!("refusal-{n}"));
    }
    let gap = device
        .store()
        .state()
        .offline_events
        .gap()
        .copied()
        .expect("losing audit history is reported");
    assert_eq!(gap.lost_tier, EventTier::Audit);
    assert_eq!(gap.lost_count, 5);
}

#[test]
fn a_long_isolation_overflows_telemetry_but_not_audit() {
    // The sizing property offline-autonomy.md §6 asks for: a realistic
    // isolation must be able to overflow telemetry without losing the record of
    // what the machine did.
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file));
    device.on_connected().unwrap();
    device.on_disconnected();

    inject_audit(&mut device, "the dose that must survive");
    for _ in 0..(TELEMETRY_CAPACITY * 4) {
        inject_telemetry(&mut device);
    }

    let events = device.store().state().offline_events.replay_events();
    assert!(
        events.iter().any(|e| e.kind == EventKind::OfflineRefused),
        "the audit event survived four times the telemetry capacity"
    );
    assert!(events.iter().any(|e| e.kind == EventKind::HistoryGap));
}

// ------------------------------------------------------- real M2 events

#[test]
fn policy_activation_is_recorded_as_a_replayable_audit_event() {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file));
    device.on_connected().unwrap();
    device.on_message(
        &Topic::Policy(rhizo_mqtt_contract::DeviceId::parse("plant-node-01").unwrap()),
        &policy_envelope(),
    );

    device.on_disconnected();
    let events: Vec<_> = batches(&device.on_connected().unwrap())
        .into_iter()
        .flat_map(|b| b.events)
        .collect();
    let activation = events
        .iter()
        .find(|e| e.kind == EventKind::PolicyActivated)
        .expect("activating a policy is part of the plant's history");
    assert_eq!(activation.tier, EventTier::Audit);
    assert_eq!(
        activation.detail,
        EventDetail::PolicyActivated { policy_version: 7 }
    );
}

#[test]
fn a_persistent_state_fault_is_recorded_as_a_lockout_event() {
    let state_file = scratch_state_file().display().to_string();
    std::fs::write(&state_file, b"not a state file").unwrap();
    let mut device = Device::new(&settings_at(&state_file));

    let events: Vec<_> = batches(&device.on_connected().unwrap())
        .into_iter()
        .flat_map(|b| b.events)
        .collect();
    let lockout = events
        .iter()
        .find(|e| e.kind == EventKind::LockoutSet)
        .expect("a lockout belongs in the plant's history, not only in a log");
    assert_eq!(lockout.tier, EventTier::Audit);
    assert!(
        matches!(&lockout.detail, EventDetail::Lockout { reason } if reason.contains("corrupt"))
    );
}

#[test]
fn a_replay_is_split_into_batches_with_complete_only_on_the_last() {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file));
    device.on_connected().unwrap();
    device.on_disconnected();
    for n in 0..(REPLAY_BATCH * 2 + 5) {
        inject_audit(&mut device, &format!("refusal-{n}"));
    }

    let batches = batches(&device.on_connected().unwrap());
    assert!(batches.len() >= 3);
    assert!(
        batches[..batches.len() - 1].iter().all(|b| !b.complete),
        "only the last batch is complete"
    );
    assert!(batches.last().unwrap().complete);
    for batch in &batches {
        batch.validate().expect("no duplicate ids within a batch");
        assert!(batch.events.len() <= REPLAY_BATCH);
    }
}

fn policy_envelope() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "v": 1,
        "kind": "device.policy",
        "message_id": rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::from_u128(1)),
        "device_id": "plant-node-01",
        "data": { "policies": [{
            "plant_id": "monstera-01",
            "policy_version": 7,
            "enabled": true,
            "actuator": {
                "actuator_id": "pump-0", "kind": "irrigation_pump",
                "dose_ml": 35.0, "max_doses_per_cycle": 3,
                "absorption_wait_ms": 900_000,
            },
            "control_measurement": {
                "kind": "soil_moisture", "point": "default",
                "trigger_below": 28.0, "resume_above": 34.0,
                "confirm_duration_ms": 1_800_000, "max_age_ms": 900_000,
            },
            "required_measurements": [
                { "kind": "tank_level", "point": "reservoir", "max_age_ms": 1_800_000 },
                { "kind": "leak_state", "point": "tray", "max_age_ms": 1_800_000 },
            ],
            "advisory_measurements": [],
            "limits": {
                "cooldown_ms": 21_600_000,
                "max_volume_per_window_ml": 300.0,
                "window_ms": 86_400_000,
            },
            "safety": {
                "require_leak_clear": true,
                "require_tank_above_percent": 15.0,
                "require_pump_healthy": true,
            },
        }] },
    }))
    .unwrap()
}
