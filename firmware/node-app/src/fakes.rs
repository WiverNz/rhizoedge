//! Host fake adapters for every trait in [`crate::ports`] (F-090-41, M9-005).
//!
//! They exist so the whole application — including SAFETY-002, -007, -011,
//! -013…-021 — can be driven with `cargo test` and no board. They are
//! configurable to produce **every failure the real hardware can**: read
//! errors, stuck values, out-of-range readings, a pump that reports success
//! without delivering, and an NVS that refuses to commit.
//!
//! No fake here knows what a GPIO is. That is the point: a host test drives the
//! full app with **no board profile involved at all**, which is what makes the
//! `app` tests board-independent by construction rather than by convention.

use std::cell::RefCell;
use std::rc::Rc;

use crate::persist::PersistedState;
use crate::ports::{
    BatterySensor, Clock, LeakSensor, Monotonic, NvsError, NvsStore, PowerRail, Pump, PumpError,
    Rng, Scale, SensorError, SoilReading, SoilSensor, TankReading, TankSensor, Watchdog,
};

/// A shared, observable call log.
///
/// Ordering assertions — "`pump_off` happened before the NVS load" — are the
/// only way a boot-sequence regression is visible before hardware exists
/// (M9-007).
pub type CallLog = Rc<RefCell<Vec<String>>>;

/// A fresh call log.
#[must_use]
pub fn call_log() -> CallLog {
    Rc::new(RefCell::new(Vec::new()))
}

/// Records one call.
pub fn record(log: &CallLog, entry: impl Into<String>) {
    log.borrow_mut().push(entry.into());
}

/// A deterministic byte source. Never used for anything cryptographic.
#[derive(Clone, Debug)]
pub struct CountingRng(u8);

impl CountingRng {
    /// A source starting from `seed`.
    #[must_use]
    pub const fn new(seed: u8) -> Self {
        Self(seed)
    }
}

impl Rng for CountingRng {
    fn fill(&mut self, out: &mut [u8]) {
        for byte in out.iter_mut() {
            *byte = self.0;
            self.0 = self.0.wrapping_add(1);
        }
    }
}

/// A pump whose behaviour every failure mode can be dialled into.
#[derive(Clone, Debug)]
pub struct FakePump {
    log: CallLog,
    /// Whether the driver reports a latched fault.
    pub faulted: bool,
    /// Whether `run_for` fails outright.
    pub drive_fails: bool,
    /// Whether it reports success without moving any water.
    ///
    /// The most valuable fake in the set: it is the failure a soil sensor and a
    /// scale exist to catch, and the one a pump can never report itself.
    pub delivers_nothing: bool,
    /// Total run time commanded.
    pub total_run_ms: u64,
    /// Whether the pump is currently energised.
    pub energised: bool,
}

impl FakePump {
    /// A healthy pump.
    #[must_use]
    pub fn new(log: CallLog) -> Self {
        Self {
            log,
            faulted: false,
            drive_fails: false,
            delivers_nothing: false,
            total_run_ms: 0,
            energised: false,
        }
    }
}

impl Pump for FakePump {
    fn run_for(&mut self, ms: u32) -> Result<(), PumpError> {
        record(&self.log, format!("pump_run_for({ms})"));
        if self.faulted {
            return Err(PumpError::Faulted);
        }
        if self.drive_fails {
            return Err(PumpError::DriveFailed);
        }
        self.energised = true;
        if !self.delivers_nothing {
            self.total_run_ms += u64::from(ms);
        }
        self.energised = false;
        Ok(())
    }

    fn off(&mut self) {
        record(&self.log, "pump_off");
        self.energised = false;
    }

    fn is_faulted(&self) -> bool {
        self.faulted
    }
}

/// A soil probe.
#[derive(Clone, Debug)]
pub struct FakeSoil {
    /// The reading to return, or `None` to fail.
    pub reading: Option<SoilReading>,
    /// Whether the value never changes regardless of the environment.
    pub stuck: bool,
}

impl FakeSoil {
    /// A probe reading `vwc_percent`.
    #[must_use]
    pub const fn reading(vwc_percent: f32) -> Self {
        Self {
            reading: Some(SoilReading {
                vwc_percent,
                temperature_c: None,
                ec_us_cm: None,
            }),
            stuck: false,
        }
    }

    /// A probe that always fails.
    #[must_use]
    pub const fn failing() -> Self {
        Self {
            reading: None,
            stuck: false,
        }
    }
}

impl SoilSensor for FakeSoil {
    fn read(&mut self) -> Result<SoilReading, SensorError> {
        self.reading.ok_or(SensorError::ReadFailed)
    }
}

/// A reservoir level sensor.
#[derive(Clone, Copy, Debug)]
pub struct FakeTank {
    /// The level to return, or `None` to fail.
    pub level_percent: Option<f32>,
}

impl FakeTank {
    /// A tank at `level_percent`.
    #[must_use]
    pub const fn at(level_percent: f32) -> Self {
        Self {
            level_percent: Some(level_percent),
        }
    }

    /// A tank sensor that always fails.
    #[must_use]
    pub const fn failing() -> Self {
        Self {
            level_percent: None,
        }
    }
}

impl TankSensor for FakeTank {
    fn read(&mut self) -> Result<TankReading, SensorError> {
        self.level_percent
            .map(|level_percent| TankReading { level_percent })
            .ok_or(SensorError::ReadFailed)
    }
}

/// A leak detector.
#[derive(Clone, Copy, Debug)]
pub struct FakeLeak {
    /// `Some(true)` means water detected; `None` means the read fails.
    pub detected: Option<bool>,
}

impl FakeLeak {
    /// A detector reporting dry.
    #[must_use]
    pub const fn clear() -> Self {
        Self {
            detected: Some(false),
        }
    }

    /// A detector reporting water.
    #[must_use]
    pub const fn wet() -> Self {
        Self {
            detected: Some(true),
        }
    }

    /// A detector that cannot be read, which the gate must treat as `Unknown`.
    #[must_use]
    pub const fn failing() -> Self {
        Self { detected: None }
    }
}

impl LeakSensor for FakeLeak {
    fn read(&mut self) -> Result<bool, SensorError> {
        self.detected.ok_or(SensorError::ReadFailed)
    }
}

/// A pot scale.
#[derive(Clone, Copy, Debug)]
pub struct FakeScale {
    /// Grams, or `None` to fail.
    pub grams: Option<f32>,
}

impl Scale for FakeScale {
    fn read(&mut self) -> Result<f32, SensorError> {
        self.grams.ok_or(SensorError::ReadFailed)
    }
}

/// A wall clock that is unsynchronised until it is told otherwise.
#[derive(Clone, Copy, Debug, Default)]
pub struct FakeClock {
    /// The wall time, or `None` when unsynchronised.
    pub now_ms: Option<i64>,
}

impl Clock for FakeClock {
    fn now_ms(&self) -> Option<i64> {
        self.now_ms
    }
}

/// A monotonic timer a test drives by hand.
#[derive(Clone, Debug, Default)]
pub struct FakeMonotonic(Rc<RefCell<u64>>);

impl FakeMonotonic {
    /// A timer starting at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advances the timer.
    pub fn advance(&self, ms: u64) {
        *self.0.borrow_mut() += ms;
    }

    /// Sets the timer.
    pub fn set(&self, ms: u64) {
        *self.0.borrow_mut() = ms;
    }
}

impl Monotonic for FakeMonotonic {
    fn monotonic_ms(&self) -> u64 {
        *self.0.borrow()
    }
}

/// An NVS store backed by memory, with a settable failure mode.
#[derive(Clone, Debug, Default)]
pub struct FakeNvs {
    stored: Rc<RefCell<Option<Vec<u8>>>>,
    /// Whether the next and subsequent commits fail.
    pub write_fails: Rc<RefCell<bool>>,
    /// Whether the stored blob should be reported as corrupt.
    pub corrupt: Rc<RefCell<bool>>,
    /// How many commits have been attempted.
    pub writes: Rc<RefCell<u32>>,
}

impl FakeNvs {
    /// An empty store, as on a first boot.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A store already holding `state`, as after a reboot.
    #[must_use]
    pub fn with(state: &PersistedState) -> Self {
        let nvs = Self::default();
        *nvs.stored.borrow_mut() = serde_json::to_vec(state).ok();
        nvs
    }

    /// Makes every subsequent commit fail.
    pub fn fail_writes(&self, fail: bool) {
        *self.write_fails.borrow_mut() = fail;
    }

    /// Simulates a corrupted region.
    pub fn corrupt(&self) {
        *self.corrupt.borrow_mut() = true;
    }

    /// Simulates a power cycle: the same bytes, a fresh handle.
    #[must_use]
    pub fn power_cycle(&self) -> Self {
        Self {
            stored: Rc::new(RefCell::new(self.stored.borrow().clone())),
            write_fails: Rc::new(RefCell::new(false)),
            corrupt: Rc::new(RefCell::new(*self.corrupt.borrow())),
            writes: Rc::new(RefCell::new(0)),
        }
    }
}

impl NvsStore for FakeNvs {
    fn load(&self) -> Option<PersistedState> {
        if *self.corrupt.borrow() {
            return None;
        }
        let bytes = self.stored.borrow().clone()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn store(&mut self, state: &PersistedState) -> Result<(), NvsError> {
        *self.writes.borrow_mut() += 1;
        if *self.write_fails.borrow() {
            return Err(NvsError::WriteFailed);
        }
        let bytes = serde_json::to_vec(state).map_err(|_| NvsError::OutOfSpace)?;
        *self.stored.borrow_mut() = Some(bytes);
        Ok(())
    }
}

/// A switched supply.
#[derive(Clone, Debug)]
pub struct FakeRail {
    log: CallLog,
    name: &'static str,
    enabled: bool,
}

impl FakeRail {
    /// A rail, initially off.
    #[must_use]
    pub const fn new(log: CallLog, name: &'static str) -> Self {
        Self {
            log,
            name,
            enabled: false,
        }
    }
}

impl PowerRail for FakeRail {
    fn enable(&mut self) {
        record(&self.log, format!("rail_enable({})", self.name));
        self.enabled = true;
    }

    fn disable(&mut self) {
        record(&self.log, format!("rail_disable({})", self.name));
        self.enabled = false;
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// A battery divider.
#[derive(Clone, Copy, Debug)]
pub struct FakeBattery {
    /// Millivolts, or `None` when the read fails.
    pub millivolts: Option<u32>,
}

impl BatterySensor for FakeBattery {
    fn read_mv(&mut self) -> Result<u32, SensorError> {
        self.millivolts.ok_or(SensorError::ReadFailed)
    }
}

/// A watchdog that counts its feeds.
#[derive(Clone, Debug, Default)]
pub struct FakeWatchdog {
    /// How many times it has been fed.
    pub feeds: u32,
    /// Whether it has been enabled.
    pub enabled: bool,
}

impl Watchdog for FakeWatchdog {
    fn feed(&mut self) {
        self.feeds += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pump_fake_reproduces_every_failure_the_real_one_can() {
        let log = call_log();
        let mut pump = FakePump::new(log.clone());
        assert!(pump.run_for(1_000).is_ok());
        assert_eq!(pump.total_run_ms, 1_000);

        pump.delivers_nothing = true;
        assert!(pump.run_for(1_000).is_ok());
        assert_eq!(pump.total_run_ms, 1_000, "reports success, moves nothing");

        pump.drive_fails = true;
        assert_eq!(pump.run_for(1), Err(PumpError::DriveFailed));

        pump.faulted = true;
        assert_eq!(pump.run_for(1), Err(PumpError::Faulted));
        assert!(pump.is_faulted());
    }

    #[test]
    fn a_failing_sensor_is_an_error_and_never_a_plausible_value() {
        assert_eq!(FakeSoil::failing().read(), Err(SensorError::ReadFailed));
        assert_eq!(FakeTank::failing().read(), Err(SensorError::ReadFailed));
        assert_eq!(FakeLeak::failing().read(), Err(SensorError::ReadFailed));
    }

    #[test]
    fn the_nvs_fake_survives_a_power_cycle_and_can_refuse_to_commit() {
        let mut nvs = FakeNvs::new();
        let state = PersistedState {
            boot_generation: 3,
            ..PersistedState::default()
        };
        assert!(nvs.store(&state).is_ok());

        let cycled = nvs.power_cycle();
        assert_eq!(cycled.load().map(|s| s.boot_generation), Some(3));

        nvs.fail_writes(true);
        assert_eq!(nvs.store(&state), Err(NvsError::WriteFailed));
    }

    #[test]
    fn a_corrupt_store_loads_nothing_rather_than_something_plausible() {
        let mut nvs = FakeNvs::new();
        assert!(nvs.store(&PersistedState::default()).is_ok());
        nvs.corrupt();
        assert!(nvs.load().is_none());
    }
}
