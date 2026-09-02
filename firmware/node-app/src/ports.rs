//! The hardware abstraction (F-090-40, F-090-41, M9-005).
//!
//! Every trait here describes *what* a peripheral does. None of them names a
//! pin, a port, or a polarity in any argument or associated type — that is the
//! board layer's job (`esp32-node/src/board/`), and a trait that mentioned
//! GPIO 4 would not be a hardware abstraction (ADR-007, amended 2026-08-28).
//!
//! [`Clock::now_ms`] returns `Option` rather than a sentinel deliberately: an
//! unsynchronised clock is not a time, and the type makes forgetting to check
//! impossible. That is the mechanism behind SAFETY-002's refusal path.

use core::fmt;

/// A sensor could not produce a usable reading.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SensorError {
    /// The bus or conversion failed.
    ReadFailed,
    /// The peripheral is not present on this build's board profile.
    NotPresent,
    /// The rail powering this sensor has not finished warming up (M9-020).
    WarmupIncomplete,
}

/// The pump could not be driven.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PumpError {
    /// The driver reported a fault and refuses to run.
    Faulted,
    /// The GPIO write failed.
    DriveFailed,
}

/// Persistent storage failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NvsError {
    /// The write did not commit. **The caller must not actuate** (F-090-34).
    WriteFailed,
    /// The stored blob failed its checksum.
    Corrupt,
    /// The region is too small for the value.
    OutOfSpace,
}

impl fmt::Display for SensorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl fmt::Display for PumpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl fmt::Display for NvsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SensorError {}
impl std::error::Error for PumpError {}
impl std::error::Error for NvsError {}

/// A soil moisture reading and, where the probe supports it, its companions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoilReading {
    /// Volumetric water content, percent.
    pub vwc_percent: f32,
    /// Soil temperature, degrees Celsius, where measurable.
    pub temperature_c: Option<f32>,
    /// Bulk electrical conductivity, where measurable.
    pub ec_us_cm: Option<f32>,
}

/// A reservoir level reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TankReading {
    /// Fill level, percent.
    pub level_percent: f32,
}

/// The water pump.
///
/// `run_for` is bounded by the caller's gate *and* by the independent run guard
/// (F-090-37, F-090-58); the implementation is never the only thing standing
/// between a request and an energised pump.
pub trait Pump {
    /// Energises for at most `ms` milliseconds.
    fn run_for(&mut self, ms: u32) -> Result<(), PumpError>;
    /// De-energises. Must be safe to call at any time, including before init.
    fn off(&mut self);
    /// Whether the driver has latched a fault.
    fn is_faulted(&self) -> bool;
}

/// The soil probe.
pub trait SoilSensor {
    /// Takes one reading.
    fn read(&mut self) -> Result<SoilReading, SensorError>;
}

/// The reservoir level sensor.
pub trait TankSensor {
    /// Takes one reading.
    fn read(&mut self) -> Result<TankReading, SensorError>;
}

/// The leak detector.
///
/// `Ok(true)` means water was detected. An `Err` is **not** "no leak": the
/// caller maps it to [`rhizo_mqtt_contract::safety::LeakState::Unknown`], which
/// the shared gate refuses (SAFETY-012).
pub trait LeakSensor {
    /// Takes one reading.
    fn read(&mut self) -> Result<bool, SensorError>;
}

/// The pot scale.
pub trait Scale {
    /// Reads grams.
    fn read(&mut self) -> Result<f32, SensorError>;
}

/// The wall clock, synchronised from the Edge over MQTT (ADR-013).
pub trait Clock {
    /// Unix epoch milliseconds, or `None` when unsynchronised.
    fn now_ms(&self) -> Option<i64>;
}

/// The monotonic timer.
///
/// Separate from [`Clock`] on purpose. Every offline rule is a duration and
/// must be measured against a source that cannot jump, and separating the two
/// traits means no call site can reach for the wall clock by accident
/// (ADR-015, SAFETY-015).
pub trait Monotonic {
    /// Milliseconds since boot. Never decreases within a boot.
    fn monotonic_ms(&self) -> u64;
}

/// Persistent device state.
pub trait NvsStore {
    /// Loads the persisted state, or `None` when absent or unreadable.
    fn load(&self) -> Option<crate::persist::PersistedState>;
    /// Commits the state durably.
    fn store(&mut self, state: &crate::persist::PersistedState) -> Result<(), NvsError>;
}

/// A switched supply for peripherals (M9-020).
///
/// The pump is deliberately **not** a `PowerRail`. It is an actuator with a
/// hard run limit, a boot-safe requirement, and an independent run guard, and
/// giving it a sensor supply's interface invites a refactor that treats them
/// alike.
pub trait PowerRail {
    /// Powers the rail.
    fn enable(&mut self);
    /// Removes power.
    fn disable(&mut self);
    /// Whether the rail is currently powered.
    fn is_enabled(&self) -> bool;
}

/// A battery voltage divider (M9-021).
///
/// Absent on hardware without one, which is why the app holds an `Option` of
/// this rather than a value: a battery field is **absent, never zero, never a
/// guess** (ADR-018 §7).
pub trait BatterySensor {
    /// Reads the supply in millivolts.
    fn read_mv(&mut self) -> Result<u32, SensorError>;
}

/// The hardware watchdog.
pub trait Watchdog {
    /// Feeds the watchdog.
    fn feed(&mut self);
}

/// Randomness for identifier generation.
pub trait Rng {
    /// Fills `out` with random bytes.
    fn fill(&mut self, out: &mut [u8]);
}
