//! RTC-retained sleep state and the wake reason (M9-019, ADR-018 §6).
//!
//! # `.rtc.data` survives deep sleep and nothing else
//!
//! It is a cache that may vanish: a power cut, a brownout, an external reset,
//! and a firmware flash all leave it holding whatever happened to be in those
//! words. That is why [`rhizo_node_app::power::RtcSleepState`] carries a
//! checksum and why [`rhizo_node_app::budget::credit_elapsed`] treats a failed
//! checksum exactly as it treats a cold boot.
//!
//! **The checksum is load-bearing, not defensive.** A corrupted RTC word that
//! was trusted would become free watering budget.
//!
//! # Why the accounting lives here rather than in NVS
//!
//! At roughly 96 wakes a day, writing the budget accumulator to flash on every
//! wake makes NVS endurance the limiting component of the device. NVS is
//! written on change and on watering; the per-wake accounting lives here
//! (F-090-60).

use rhizo_node_app::power::{RtcSleepState, WakeReason};

/// The RTC-retained words.
///
/// `#[link_section = ".rtc.data"]` places this in the RTC fast memory the deep
/// sleep domain keeps powered. It is a fixed-shape POD: a growing struct here
/// is a silent corruption waiting for a firmware upgrade, because the previous
/// image's bytes will still be sitting in those addresses.
#[link_section = ".rtc.data"]
static mut RTC_SLEEP_STATE: RtcWords = RtcWords {
    slept_at_ms: 0,
    boot_generation: 0,
    cooldown_remaining_ms: 0,
    checksum: 0,
};

/// The retained words, in a layout that does not change.
#[repr(C)]
#[derive(Clone, Copy)]
struct RtcWords {
    slept_at_ms: u64,
    boot_generation: u64,
    cooldown_remaining_ms: u64,
    checksum: u32,
}

/// Reads the retained sleep state.
///
/// Returns the words as stored, checksum included. The caller — and only the
/// caller — decides what a failed checksum means, because that decision is
/// [`rhizo_node_app::budget::credit_elapsed`] and belongs in the host-testable
/// crate where its branches can actually be exercised.
#[must_use]
pub fn read() -> RtcSleepState {
    // SAFETY: single-threaded access from the wake path, before any task that
    // could touch it is started. The read is of plain data with no invariants.
    let words = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(RTC_SLEEP_STATE)) };
    RtcSleepState {
        slept_at_ms: words.slept_at_ms,
        boot_generation: words.boot_generation,
        cooldown_remaining_ms: words.cooldown_remaining_ms,
        checksum: words.checksum,
    }
}

/// Writes the retained sleep state, sealed with its checksum.
pub fn write(state: &RtcSleepState) {
    let words = RtcWords {
        slept_at_ms: state.slept_at_ms,
        boot_generation: state.boot_generation,
        cooldown_remaining_ms: state.cooldown_remaining_ms,
        checksum: state.checksum,
    };
    // SAFETY: as above. Written immediately before `esp_deep_sleep`, from the
    // one call site, with no other task running.
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(RTC_SLEEP_STATE), words);
    }
}

/// The reason this boot happened, mapped to the contract's vocabulary.
///
/// **Exhaustive with no catch-all that means "timer".** An unrecognised reset
/// reason maps to [`WakeReason::Unknown`], which credits zero elapsed time —
/// uncertainty must not choose the branch that grants budget.
#[must_use]
pub fn wake_reason() -> WakeReason {
    // SAFETY: `esp_reset_reason` reads a latched register and has no
    // preconditions.
    let reason = unsafe { esp_idf_sys::esp_reset_reason() };
    match reason {
        esp_idf_sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP => deep_sleep_wake_reason(),
        esp_idf_sys::esp_reset_reason_t_ESP_RST_POWERON
        | esp_idf_sys::esp_reset_reason_t_ESP_RST_BROWNOUT
        | esp_idf_sys::esp_reset_reason_t_ESP_RST_SW
        | esp_idf_sys::esp_reset_reason_t_ESP_RST_EXT => WakeReason::ColdBoot,
        esp_idf_sys::esp_reset_reason_t_ESP_RST_INT_WDT
        | esp_idf_sys::esp_reset_reason_t_ESP_RST_TASK_WDT
        | esp_idf_sys::esp_reset_reason_t_ESP_RST_WDT => WakeReason::Watchdog,
        _ => WakeReason::Unknown,
    }
}

/// Which wake source ended a deep sleep.
///
/// A deep-sleep reset is only a *timer* wake if the timer is what woke it.
/// Anything else — an external pin, an unrecognised source — is not, and
/// therefore credits nothing.
fn deep_sleep_wake_reason() -> WakeReason {
    // SAFETY: reads a latched cause register; no preconditions.
    let cause = unsafe { esp_idf_sys::esp_sleep_get_wakeup_cause() };
    match cause {
        esp_idf_sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_TIMER => WakeReason::Timer,
        esp_idf_sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_GPIO
        | esp_idf_sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_EXT1 => WakeReason::External,
        _ => WakeReason::Unknown,
    }
}
