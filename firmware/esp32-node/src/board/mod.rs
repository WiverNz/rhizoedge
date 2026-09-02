//! The board layer — **the only place a GPIO number, a pin polarity, or a
//! board-specific peripheral construction may appear** (ADR-007, F-090-44).
//!
//! # Compile-time, not runtime
//!
//! Exactly one board profile is selected per build, by a Cargo feature. Zero or
//! two is a `compile_error!` naming the available profiles — not a runtime
//! panic and not a silent default. A device that boots with the wrong pin map
//! drives the pump GPIO as something else, which is the one failure class this
//! project refuses to discover on hardware.
//!
//! A runtime pin table was rejected for the same reason ADR-011 keeps hard
//! limits out of messages: it would make "which pin energises the pump" a
//! configurable value. It also buys nothing, because the board is soldered in
//! place before the firmware is flashed.
//!
//! # What is above this line cannot see below it
//!
//! Everything above receives already-constructed trait objects — `Pump`,
//! `PowerRail`, and the rest of `rhizo_node_app::ports` — and cannot observe
//! which board it is running on. `rhizo-node-app` does not depend on this crate
//! at all, so that is a fact about the dependency graph rather than a
//! convention. `../node-app/tests/board_isolation.rs` additionally checks the
//! source text, because a convention nobody checks is not a boundary.
//!
//! # Adding a board
//!
//! A new file here, a feature entry in `Cargo.toml`, and a matrix line in CI.
//! No change to application, safety, sensor, pump, or networking code, and no
//! change to the MQTT contract, identity semantics, configuration semantics, or
//! the NVS data model (F-090-47).

#[cfg(all(feature = "board-devkitm1", feature = "board-xiao-esp32c3"))]
compile_error!(
    "exactly one board profile must be selected, and two are: \
     `board-devkitm1` and `board-xiao-esp32c3`. \
     Available board profiles: `board-devkitm1` (Espressif ESP32-C3-DEVKITM-1-N4X, \
     the reference board), `board-xiao-esp32c3` (Seeed XIAO ESP32-C3, reserved). \
     Build with `--no-default-features --features <one profile>`."
);

#[cfg(not(any(feature = "board-devkitm1", feature = "board-xiao-esp32c3")))]
compile_error!(
    "no board profile is selected. \
     Available board profiles: `board-devkitm1` (Espressif ESP32-C3-DEVKITM-1-N4X, \
     the reference board), `board-xiao-esp32c3` (Seeed XIAO ESP32-C3, reserved). \
     Build with `--features <one profile>`; there is deliberately no default \
     at this layer, because a board that boots with the wrong pin map drives \
     the pump GPIO as something else."
);

#[cfg(feature = "board-xiao-esp32c3")]
compile_error!(
    "`board-xiao-esp32c3` is a reserved profile name and has no pin map yet. \
     ADR-007 names the Seeed XIAO ESP32-C3 as a *candidate* battery-deployment \
     board; it has not been purchased and nothing about it has been measured, \
     so M9 ships the board seam rather than an unverifiable pin map. Writing \
     one is a new file in `src/board/` and a matrix line in CI -- and no change \
     to application, safety, sensor, pump, or networking code."
);

#[cfg(feature = "board-devkitm1")]
mod devkitm1;

#[cfg(feature = "board-devkitm1")]
pub use devkitm1::Devkitm1 as ActiveBoard;

use esp_idf_hal::modem::Modem;
use esp_idf_hal::peripherals::Peripherals;
use rhizo_node_app::ports::{PowerRail, Pump};

/// The name of the board profile this image was built for.
///
/// Reported in `device.status` as a diagnostic. It is not configuration and
/// nothing decides anything from it; it exists so that a device on a bench can
/// be asked which pin map it is running.
pub const PROFILE: &str = ActiveBoard::PROFILE;

/// What every board profile must supply.
///
/// The trait carries no pin, port, or polarity in any signature: a trait that
/// mentioned GPIO 4 would not be a hardware abstraction. Each method returns an
/// already-constructed adapter, so the caller receives a `Pump` rather than the
/// information needed to build one.
pub trait BoardProfile: Sized {
    /// The pump driver, already constructed and already de-energised.
    type Pump: Pump;
    /// A switched supply.
    type Rail: PowerRail;

    /// The profile's name.
    const PROFILE: &'static str;

    /// Claims the peripherals and constructs every adapter.
    ///
    /// # Errors
    ///
    /// If a peripheral cannot be claimed. There is no recovery: a device that
    /// cannot construct its pump cannot safely do anything else.
    fn take(peripherals: Peripherals) -> Result<Board<Self>, esp_idf_sys::EspError>;
}

/// Everything a board hands upward.
///
/// Note what is **not** here: no pin numbers, no polarity, no peripheral
/// handles. Consumers get behaviour.
pub struct Board<P: BoardProfile> {
    /// The pump, de-energised.
    pub pump: P::Pump,
    /// The sensor supply, where this board has one.
    pub sensor_rail: Option<P::Rail>,
    /// The RS485 transceiver supply, where this board has one.
    ///
    /// `None` on a board with no RS485 capability. The sampling code asks the
    /// board rather than assuming, so a device with an analogue probe never
    /// enables a transceiver it has no use for (M9-020).
    pub rs485_rail: Option<P::Rail>,
    /// The radio.
    ///
    /// Handed up with the rest rather than claimed separately, because
    /// `Peripherals::take()` may only be called once and the board is what
    /// calls it. Every ESP32-C3 has exactly one, so this is not board detail —
    /// no profile can differ about it, and nothing above can name a pin
    /// through it.
    pub modem: Modem<'static>,
}

/// Claims the peripherals and builds the active board profile.
///
/// # Errors
///
/// If a peripheral cannot be claimed.
pub fn take() -> Result<Board<ActiveBoard>, esp_idf_sys::EspError> {
    let peripherals = Peripherals::take()?;
    ActiveBoard::take(peripherals)
}
