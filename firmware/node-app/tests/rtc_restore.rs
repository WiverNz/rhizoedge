//! Retained sleep accounting uses measured elapsed time exactly once.
#![allow(clippy::unwrap_used)]
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
