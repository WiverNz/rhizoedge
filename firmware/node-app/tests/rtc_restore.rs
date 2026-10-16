//! Retained sleep accounting uses measured elapsed time exactly once.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use rhizo_mqtt_contract::{Envelope, payload::OfflinePolicySet};
use rhizo_node_app::{
    budget::restore_sleep_accounting,
    persist::{PersistedState, VersionedPolicy},
    power::{RtcSleepState, WakeReason},
};
fn seeded() -> PersistedState {
    let mut state = PersistedState {
        boot_generation: 7,
        ..PersistedState::default()
    };
    state.offline_runtime.budget_used_ml = 40.0;
    state.offline_runtime.window_elapsed_ms = 30_000;
    state.offline_runtime.cooldown_remaining_ms = 120_000;
    state.daily.delivered_ml = 40.0;
    let mut policy = Envelope::<OfflinePolicySet>::from_json(include_bytes!(
        "../../../test/fixtures/protocol/valid/policy-disabled.json"
    ))
    .unwrap()
    .data;
    policy.policies[0].limits.window_ms = 120_000;
    state.policy_active = Some(VersionedPolicy::seal(8, policy));
    state
}
#[test]
fn timer_wake_advances_shared_window_and_cooldown_without_replenishing_early() {
    let mut state = seeded();
    let rtc = RtcSleepState::seal(1_000, 7, 120_000);
    assert_eq!(
        restore_sleep_accounting(&mut state, WakeReason::Timer, Some(&rtc), 61_000).0,
        60_000
    );
    assert_eq!(state.offline_runtime.budget_used_ml, 40.0);
    assert_eq!(state.offline_runtime.window_elapsed_ms, 90_000);
    assert_eq!(state.offline_runtime.cooldown_remaining_ms, 60_000);
    assert_eq!(state.daily.delivered_ml, 40.0);
    // begin_boot advances this persisted generation before returning to work.
    state.boot_generation += 1;
    let before = state.clone();
    assert_eq!(
        restore_sleep_accounting(&mut state, WakeReason::Timer, Some(&rtc), 91_000).0,
        0
    );
    assert_eq!(state, before);
}
#[test]
fn only_an_observed_full_window_can_replenish() {
    let mut state = seeded();
    let rtc = RtcSleepState::seal(1_000, 7, 120_000);
    restore_sleep_accounting(&mut state, WakeReason::Timer, Some(&rtc), 91_000);
    assert_eq!(state.offline_runtime.budget_used_ml, 0.0);
    assert_eq!(state.offline_runtime.window_elapsed_ms, 0);
    assert_eq!(state.offline_runtime.cooldown_remaining_ms, 30_000);
}
#[test]
fn non_timer_corrupt_stale_and_backward_retention_grant_nothing() {
    for reason in [
        WakeReason::ColdBoot,
        WakeReason::Watchdog,
        WakeReason::External,
        WakeReason::Unknown,
    ] {
        let mut state = seeded();
        let before = state.clone();
        let rtc = RtcSleepState::seal(1_000, 7, 120_000);
        assert_eq!(
            restore_sleep_accounting(&mut state, reason, Some(&rtc), 999_999).0,
            0
        );
        assert_eq!(state, before);
    }
    for (mut rtc, now) in [
        (RtcSleepState::seal(1_000, 7, 120_000), 999_999),
        (RtcSleepState::seal(1_000, 6, 120_000), 999_999),
        (RtcSleepState::seal(1_000, 7, 119_999), 999_999),
        (RtcSleepState::seal(1_000, 7, 120_000), 500),
    ] {
        if now == 999_999 && rtc.boot_generation == 7 && rtc.cooldown_remaining_ms == 120_000 {
            rtc.checksum ^= 1;
        }
        let mut state = seeded();
        let before = state.clone();
        assert_eq!(
            restore_sleep_accounting(&mut state, WakeReason::Timer, Some(&rtc), now).0,
            0
        );
        assert_eq!(state, before);
    }
}

#[test]
fn missing_retention_and_invalid_policy_never_replenish_a_window() {
    let mut state = seeded();
    let before = state.clone();
    assert_eq!(
        restore_sleep_accounting(&mut state, WakeReason::Timer, None, 999999).0,
        0
    );
    assert_eq!(state, before);
    state.policy_active.as_mut().unwrap().crc32 ^= 1;
    let rtc = RtcSleepState::seal(1000, 7, 120000);
    restore_sleep_accounting(&mut state, WakeReason::Timer, Some(&rtc), 91000);
    assert_eq!(state.offline_runtime.budget_used_ml, 40.0);
    assert_eq!(state.offline_runtime.window_elapsed_ms, 30000);
    assert_eq!(state.offline_runtime.cooldown_remaining_ms, 30000);
}

/// A durable result's old monotonic publication instant is meaningless after
/// reboot; resetting scheduling must preserve its identity and accounting.
#[test]
fn reboot_makes_unacknowledged_results_due_without_changing_accounting() {
    use rhizo_mqtt_contract::payload::CommandResult;
    use rhizo_node_app::{fakes::CountingRng, identity::begin_boot};
    let result: CommandResult = serde_json::from_value(serde_json::json!({
        "command_id":"018fd7b1-4c2e-7f10-a3b8-9d1e2f304050",
        "status":"completed", "requested_ml":4.0, "delivered_ml":4.0,
        "duration_ms":1000, "clamped":false, "delivered_today_ml":4.0,
        "origin":"edge_command"
    }))
    .expect("valid result");
    let mut state = rhizo_node_app::persist::PersistedState::default();
    state.daily.delivered_ml = 4.0;
    state.pending_results.insert(result.clone()).expect("space");
    state
        .pending_results
        .mark_published(result.command_id, 86_400_000);
    let bytes = serde_json::to_vec(&state).expect("encode");
    let mut restored = serde_json::from_slice(&bytes).expect("decode");
    begin_boot(&mut restored, None, &mut CountingRng::new(1));
    assert_eq!(restored.daily.delivered_ml, 4.0);
    assert_eq!(restored.pending_results.due(0), vec![&result]);
    assert_eq!(restored.pending_results.entries()[0].publish_count, 1);
}
