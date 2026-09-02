//! Deep sleep — **the one `esp_deep_sleep` call site** (F-090-51).
//!
//! There is exactly one function in this firmware that can put the chip to
//! sleep, it lives in this file, and it takes a
//! [`rhizo_node_app::power::WakeAction`] rather than a duration. A caller
//! cannot ask for sleep; it can only pass on what the wake state machine
//! decided, and the only phase that yields `WakeAction::Sleep` is one the
//! machine will not enter while an awake hold is outstanding or before the
//! sleep announcement has been acknowledged.
//!
//! That is the difference between "unlikely" and "unrepresentable". A
//! `deep_sleep()` reachable from a command handler or an error path is how a
//! device sleeps with the pump energised.
//!
//! `../node-app/tests/board_isolation.rs` asserts that this file is the only
//! one in `src/` that names `esp_deep_sleep`.

use rhizo_node_app::power::{RtcSleepState, WakeAction};

/// Why sleep did not happen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotSleeping {
    /// The wake machine asked for something other than sleep.
    ///
    /// The only way to reach the call below is to be handed
    /// [`WakeAction::Sleep`], which the machine produces from exactly one
    /// phase.
    NotAuthorised,
}

/// Enters deep sleep for `wake_interval_seconds`, never returning on success.
///
/// The RTC state is sealed and written **immediately before** the call, so the
/// `slept_at_ms` it records is as close as possible to the instant the counter
/// stops being observed. `esp_deep_sleep` does not return: execution resumes at
/// the reset vector.
///
/// # Errors
///
/// [`NotSleeping::NotAuthorised`] if the caller passed any action but
/// [`WakeAction::Sleep`]. There is no other way in.
pub fn enter(
    action: WakeAction,
    wake_interval_seconds: u32,
    boot_generation: u64,
    cooldown_remaining_ms: u64,
) -> Result<core::convert::Infallible, NotSleeping> {
    if action != WakeAction::Sleep {
        return Err(NotSleeping::NotAuthorised);
    }

    let slept_at_ms = crate::hal::clock::monotonic_ms();
    let sealed = RtcSleepState::seal(slept_at_ms, boot_generation, cooldown_remaining_ms);
    crate::hal::rtc_store::write(&sealed);

    let micros = u64::from(wake_interval_seconds).saturating_mul(1_000_000);
    log::info!("entering deep sleep for {wake_interval_seconds}s");

    // SAFETY: both calls are ESP-IDF entry points with no Rust-side
    // preconditions. `esp_deep_sleep` does not return.
    unsafe {
        esp_idf_sys::esp_sleep_enable_timer_wakeup(micros);
        esp_idf_sys::esp_deep_sleep(micros);
    }

    // `esp_deep_sleep` is `-> !` in C and the binding is `-> !` in Rust, so
    // control never reaches here. The `#[allow]` says so rather than leaving a
    // warning for a reader to wonder about.
    #[allow(unreachable_code)]
    {
        unreachable!("esp_deep_sleep does not return")
    }
}
