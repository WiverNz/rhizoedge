//! The device core.
//!
//! A sans-I/O state machine: it consumes connection events, inbound payloads,
//! and elapsed virtual time, and it produces [`Publication`]s. It never touches
//! a socket, a clock, or a file directly.
//!
//! That shape is not decoration. Every protocol and safety property in PRD 020
//! is then assertable in a unit test with no broker and no sleeping, and the
//! MQTT driver reduces to "hand it events, publish what comes back" — which is
//! the part that would otherwise be untestable.

use rhizo_mqtt_contract::payload::{
    ActuatorState, CommandOrigin, CommandResult, CommandStatus, Connectivity, DeviceConfig,
    DeviceStatus, DeviceStatusValue, EdgeTime, MeasurementKind, ReportedLimits, TelemetryBatch,
};
use rhizo_mqtt_contract::payload::{EventAck, EventDetail, EventKind, EventTier};
use rhizo_mqtt_contract::safety::{
    FIRMWARE_MAX_DAILY_ML, FIRMWARE_MAX_ML_PER_RUN, FIRMWARE_MAX_RUN_SECONDS, LeakState,
};
use rhizo_mqtt_contract::{
    BootId, DecodeError, DeviceId, Envelope, EventId, PROTOCOL_VERSION, Topic,
};
use std::collections::BTreeMap;

use crate::buffer::{AckOutcome, Buffered};
use crate::capabilities::Capabilities;
use crate::cli::Cli;
use crate::clock::MonotonicClock;
use crate::config::{ConfigOutcome, EffectiveConfig};
use crate::envelope::{Identity, Publication};
use crate::environment::Environment;
use crate::fault::FaultSet;
use crate::isolation::IsolationState;
use crate::offline_state::RecentSamples;
use crate::pump::{Pump, PumpStep};
use crate::state::{PersistentStateFault, StateStore};
use crate::telemetry::{self, SensorHealth, TelemetryRing};
use crate::time_sync::{TimeSync, UNSYNCED_STATUS_INTERVAL_MS};

/// The firmware version the simulator reports.
///
/// It reports the crate version rather than inventing one, so a device on the
/// broker can be traced back to a build.
pub const FIRMWARE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The simulated free heap, in bytes.
///
/// A plausible constant rather than a model: nothing in the system reasons
/// about it, and a fabricated downward trend would invite something to.
const SIMULATED_FREE_HEAP_BYTES: u32 = 142_336;

/// The simulated Wi-Fi signal strength, in dBm.
const SIMULATED_RSSI_DBM: i16 = -58;

/// How much observed virtual time may accumulate before the offline runtime
/// state is written to the store.
///
/// One virtual minute. Discarding up to a minute of observed time on a crash is
/// conservative in the only direction that matters: the cooldown stays longer
/// and the budget window shorter than reality, so a crash can never buy a dose.
const OFFLINE_RUNTIME_PERSIST_MS: u64 = 60_000;

/// The device.
#[derive(Debug)]
pub struct Device {
    identity: Identity,
    monotonic: MonotonicClock,
    time_sync: TimeSync,
    config: EffectiveConfig,
    connected: bool,
    /// Monotonic instant of the last status publication, for the heartbeat and
    /// for bounding the unsynchronised republication rate.
    last_status_ms: u64,
    /// The synchronisation state at the last status publication, so a
    /// transition can be announced rather than waiting for the next heartbeat.
    last_reported_sync: bool,
    /// What this device declares it can sense and actuate.
    capabilities: Capabilities,
    /// The simulated pot, reservoir, and probes.
    environment: Environment,
    /// Cycles buffered while disconnected.
    telemetry_ring: TelemetryRing,
    /// Monotonic instant of the last sampling cycle.
    last_sample_ms: u64,
    /// Leak state at the last cycle, so a change can force one immediately.
    last_leak: LeakState,
    /// The actuator state last published, so only changes are published.
    last_actuator: Option<ActuatorState>,
    /// Whether the actuator is currently energised.
    actuator_active: bool,
    /// Duration of the previous actuator run.
    actuator_last_run_ms: Option<u32>,
    /// Whether the actuator reports a hardware fault.
    actuator_faulted: bool,
    /// The NVS-equivalent store: everything that survives a restart.
    store: StateStore,
    /// The pump mechanism. It decides nothing; it only runs.
    pump: Pump,
    /// Diagnostic detail carried from the accepted command to its result.
    pub(crate) pending_detail: Option<String>,
    /// Whether a hard limit changed the accepted request.
    pub(crate) pending_clamped: bool,
    /// Who authorised the dose in flight.
    pub(crate) pending_origin: CommandOrigin,
    /// Whether a running pump actually moves water.
    pump_delivers: bool,
    /// The faults currently injected.
    faults: FaultSet,
    /// The settings this device was started with, so it can restart itself.
    settings: std::sync::Arc<Cli>,
    /// The readings frozen by `stuck-sensor`, captured when it was enabled.
    stuck_readings: Option<BTreeMap<String, rhizo_mqtt_contract::payload::MeasurementValue>>,
    /// Set when a fault restarted the device, so the run loop rebuilds the
    /// broker connection the way a real restart would.
    restart_notice: bool,
    /// Virtual milliseconds of isolation still owed by the `disconnect` fault.
    isolation_remaining_ms: u64,
    /// The device's own view of whether it can reach the Edge.
    isolation: IsolationState,
    /// Per-kind last readings, for the staleness checks M6-019 will make.
    recent_samples: RecentSamples,
    /// The most recent policy rejection, kept so it is observable.
    pub(crate) last_policy_rejection: Option<crate::policy::PolicyRejection>,
    /// Observed monotonic time not yet folded into the persisted runtime state.
    unpersisted_runtime_ms: u64,
    /// What the most recent `event.ack` did.
    last_ack_outcome: Option<AckOutcome>,
    /// The wake cycle. Always-on devices carry an inert one.
    power: crate::power::PowerState,
    /// Set when the device has announced its sleep and is ready to leave the
    /// broker, so the run loop drops the socket *after* the publication.
    sleep_notice: bool,
    /// Whether that sleep was announced. An announced sleep leaves cleanly so
    /// the retained announcement survives; an unannounced one drops the socket
    /// so the will fires, which is the whole point of the fault.
    sleep_announced: bool,
    /// The offline refusal most recently recorded, so a persistent condition
    /// buffers one audit event rather than one per tick (M6-019).
    last_offline_refusal: Option<rhizo_policy::RefuseReason>,
    /// Monotonic instant of the last `command.result` publication attempt.
    ///
    /// A result is retried until the **edge** acknowledges it, not until the
    /// broker does, so the device needs its own retry clock. In memory rather
    /// than in the store because losing it costs one early retry and nothing
    /// else, while a reboot republishes from `pending_results` anyway.
    last_result_publish_ms: u64,
    /// Timer wakes whose validated monotonic elapsed time was credited.
    credited_timer_wakes: u64,
    /// Non-timer resets that conservatively credited no elapsed time.
    zero_credit_resets: u64,
    /// Explicit RTC checksum failures that conservatively credited no time.
    zero_credit_checksum_failures: u64,
}

impl Device {
    /// Builds a device from validated command-line settings.
    #[must_use]
    pub fn new(cli: &Cli) -> Self {
        Self::with_store(cli, StateStore::load(cli.resolved_state_file()))
    }

    /// Rebuilds this device as a fresh boot of the same configuration.
    ///
    /// A real restart: the state file is reloaded, the boot count advances, the
    /// `boot_id` is fresh, the sequence restarts, the pump is off, and a dose
    /// that was in flight is reported as interrupted. Anything less would make
    /// `--fault restart-mid-dose` a simulation of a restart rather than one.
    ///
    /// **The plant does not reboot.** The soil, the reservoir, and the pot keep
    /// their state across a restart, because they are outside the device. A
    /// restart that reset moisture to its starting value would hide every
    /// consequence of the dose that was interrupted — the one thing the
    /// interrupted-dose path exists to make visible.
    ///
    /// Injected faults carry across too, for the same reason: a leak does not
    /// dry up because the device rebooted. One-shot faults disable themselves
    /// before restarting, so a `--fault restart` cannot loop forever.
    pub fn restart(&mut self) {
        let settings = std::sync::Arc::clone(&self.settings);
        let environment = self.environment.clone();
        let faults = self.faults.clone();
        let credited_timer_wakes = self.credited_timer_wakes;
        let zero_credit_resets = self.zero_credit_resets.saturating_add(1);
        let zero_credit_checksum_failures = self.zero_credit_checksum_failures;
        *self = Self::new(&settings);
        self.environment = environment;
        self.faults = faults;
        self.credited_timer_wakes = credited_timer_wakes;
        self.zero_credit_resets = zero_credit_resets;
        self.zero_credit_checksum_failures = zero_credit_checksum_failures;
        self.apply_faults();
        self.restart_notice = true;
    }

    /// Whether a fault restarted the device since this was last asked.
    ///
    /// The run loop uses it to rebuild the connection. Consuming the flag means
    /// a restart is acted on once, not on every poll.
    pub const fn take_restart_notice(&mut self) -> bool {
        let notice = self.restart_notice;
        self.restart_notice = false;
        notice
    }

    /// Builds a device around an already-loaded state store.
    #[must_use]
    pub fn with_store(cli: &Cli, mut store: StateStore) -> Self {
        let invalid_policy = store
            .state()
            .policy_active
            .as_ref()
            .is_some_and(|policy| !policy.verify())
            || store
                .state()
                .policy_staging
                .as_ref()
                .is_some_and(|policy| !policy.verify());
        if invalid_policy {
            let _ = store.mutate(|state| {
                state.policy_active = None;
                state.policy_staging = None;
                state.applied_policy_versions.clear();
            });
        }
        let environment = Environment::from_cli(cli);
        let mut config = EffectiveConfig::from_cli(cli);
        // A restart resumes the configuration it was running, so a device that
        // reboots does not silently regress to its command-line defaults and
        // then accept a republished retained config it had already applied.
        config.applied_version = store.state().applied_config_version;
        let boot_fault = store.fault().map(|fault| {
            tracing::error!(
                reason = %fault.reason,
                detail = %fault.detail,
                "starting in diagnostic mode: sensing and reporting continue, actuation is disabled"
            );
            fault.reason.clone()
        });
        let mut device = Self {
            identity: Identity::new(cli.device_id.clone()),
            monotonic: MonotonicClock::start(),
            time_sync: TimeSync::default(),
            config,
            connected: false,
            last_status_ms: 0,
            last_reported_sync: false,
            capabilities: Capabilities::from_cli(cli),
            last_leak: environment.tank.leak(),
            environment,
            telemetry_ring: TelemetryRing::default(),
            last_sample_ms: 0,
            last_actuator: None,
            actuator_active: false,
            actuator_last_run_ms: None,
            actuator_faulted: false,
            store,
            pump: Pump::new(),
            pending_detail: None,
            pending_clamped: false,
            pending_origin: CommandOrigin::EdgeCommand,
            pump_delivers: true,
            faults: FaultSet::from_flags(&cli.faults),
            settings: std::sync::Arc::new(cli.clone()),
            stuck_readings: None,
            restart_notice: false,
            isolation_remaining_ms: 0,
            isolation: IsolationState::default(),
            recent_samples: RecentSamples::default(),
            last_policy_rejection: None,
            unpersisted_runtime_ms: 0,
            last_offline_refusal: None,
            last_ack_outcome: None,
            last_result_publish_ms: 0,
            credited_timer_wakes: 0,
            zero_credit_resets: 0,
            zero_credit_checksum_failures: 0,
            power: if cli.power_mode.is_battery() {
                crate::power::PowerState::battery(
                    cli.wake_interval_seconds,
                    cli.awake_budget_seconds,
                    cli.sensor_warmup_ms,
                )
            } else {
                crate::power::PowerState::always_on()
            },
            sleep_notice: false,
            sleep_announced: false,
        };
        // A boot always begins with the pump off, and a dose that was in flight
        // when the power went is reported before anything else happens
        // (SAFETY-011).
        device.report_interrupted_dose();
        device.apply_faults();
        if let Some(reason) = boot_fault {
            // An audit event, so the lockout appears in the plant's history
            // rather than only in a log nobody reads.
            device.record_event(
                EventTier::Audit,
                EventKind::LockoutSet,
                EventDetail::Lockout { reason },
            );
        }
        if invalid_policy {
            device.last_policy_rejection = Some(crate::policy::PolicyRejection::Malformed(
                "persisted policy checksum mismatch",
            ));
            device.record_event(
                EventTier::Audit,
                EventKind::OfflineRefused,
                EventDetail::Refused {
                    reason: "policy_invalid".to_owned(),
                },
            );
        }
        device
    }

    /// Mutable access to the persistent store.
    pub(crate) const fn store_mut(&mut self) -> &mut StateStore {
        &mut self.store
    }

    /// Whether the pump is currently energised.
    #[must_use]
    pub const fn pump_running(&self) -> bool {
        self.pump.is_running()
    }

    /// Makes the pump fail to de-energise, for the `pump-stuck-on` fault.
    pub const fn set_pump_stuck_on(&mut self, stuck: bool) {
        self.pump.set_stuck_on(stuck);
    }

    /// Whether the device samples a measurement kind at all.
    pub(crate) fn samples_kind(&self, kind: &MeasurementKind) -> bool {
        self.capabilities
            .sensors()
            .iter()
            .any(|sensor| sensor.kinds.iter().any(|k| k == kind))
    }

    /// Starts the pump for an already-authorised run.
    ///
    /// Called from exactly one place: the accept arm of the shared gate. It
    /// takes an authorisation, never a request — there is nothing here that
    /// could decide to run.
    pub(crate) fn start_pump(
        &mut self,
        command_id: rhizo_mqtt_contract::CommandId,
        run_ms: u32,
        effective_ml: f32,
    ) -> Vec<Publication> {
        self.pump.start(command_id, run_ms, effective_ml);
        self.actuator_active = true;
        // The actuator state change is published on the next tick, through the
        // same change-detection path as any other transition.
        Vec::new()
    }

    /// Reports a dose that a restart interrupted.
    ///
    /// `delivered_ml: null` — the volume is genuinely unknown, and inventing a
    /// number would put a fiction into the plant's history. The daily total is
    /// credited the **full requested volume** instead, because a device that
    /// cannot know what it delivered must assume the worst (protocol §5.10).
    fn report_interrupted_dose(&mut self) {
        let Some(dose) = self.store.state().in_flight_dose.clone() else {
            return;
        };
        tracing::warn!(
            command_id = %dose.command_id,
            requested_ml = dose.requested_ml,
            "a dose was in flight at the last shutdown; reporting it as interrupted"
        );
        let wall = self.wall_now();
        if let Err(e) = self.store.mutate(|state| {
            state.credit_delivery(dose.requested_ml, wall);
            state.in_flight_dose = None;
        }) {
            tracing::error!(error = %e, "could not clear the interrupted dose");
        }
        let result = CommandResult {
            command_id: dose.command_id,
            status: CommandStatus::Interrupted,
            requested_ml: dose.requested_ml,
            delivered_ml: None,
            duration_ms: None,
            clamped: false,
            reason: None,
            delivered_today_ml: self.store.state().delivered_today_ml,
            origin: CommandOrigin::EdgeCommand,
            detail: Some(String::from("device restarted during actuation")),
        };
        // Recorded in the ring so a redelivery of the same command is
        // deduplicated rather than run a second time, and queued so the edge
        // learns about it as soon as there is a connection.
        let _ = self.record_and_queue(result);
    }

    /// The persistent store.
    #[must_use]
    pub const fn store(&self) -> &StateStore {
        &self.store
    }

    /// Monotonic milliseconds since this boot, for the retry clocks.
    pub(crate) const fn elapsed_ms(&self) -> u64 {
        self.monotonic.elapsed_ms()
    }

    /// When results were last handed to the broker.
    pub(crate) const fn last_result_publish_ms(&self) -> u64 {
        self.last_result_publish_ms
    }

    /// Records a publication attempt, restarting the retry interval.
    pub(crate) const fn note_result_publish(&mut self) {
        self.last_result_publish_ms = self.monotonic.elapsed_ms();
    }

    /// How many results the edge has not yet acknowledged.
    ///
    /// Public so a test can assert the durability property directly: a result
    /// that has been published but not acknowledged is still held.
    #[must_use]
    pub fn unacknowledged_results(&self) -> usize {
        self.store.state().pending_results.len()
    }

    /// Seeds persisted state directly, for tests only.
    ///
    /// Compiled only under `cfg(test)` and for this crate's own test binaries.
    /// It exists so a test can start from a device that has *already* been
    /// running for a day — a cooldown part-served, a budget part-spent — which
    /// is otherwise reachable only by simulating a day. It cannot start a dose:
    /// it reaches the store, and the only thing that moves the pump is the
    /// accept arm of the shared gate.
    ///
    /// # Errors
    ///
    /// Returns the write failure.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn store_mut_for_test<T>(
        &mut self,
        change: impl FnOnce(&mut crate::state::PersistentState) -> T,
    ) -> Result<T, crate::state::StateError> {
        self.store.mutate(change)
    }

    /// Whether actuation is permitted at all.
    ///
    /// `false` while a persistent-state fault stands. Monitoring, telemetry, and
    /// diagnostics continue regardless — a device that cannot trust its stored
    /// safety state is still a working sensor node, it simply must not water.
    #[must_use]
    pub const fn actuation_permitted(&self) -> bool {
        self.store.actuation_permitted()
    }

    /// The persistent-state fault, if the stored state could not be trusted.
    #[must_use]
    pub const fn persistent_state_fault(&self) -> Option<&PersistentStateFault> {
        self.store.fault()
    }

    /// Volume delivered against the compile-time daily cap.
    #[must_use]
    pub const fn delivered_today_ml(&self) -> f32 {
        self.store.state().delivered_today_ml
    }

    /// Volume charged to the current offline-policy rolling window.
    #[must_use]
    pub const fn offline_budget_used_ml(&self) -> f32 {
        self.store
            .state()
            .offline_runtime
            .budget_window
            .delivered_ml
    }

    /// Persisted cooldown remaining for offline autonomy.
    #[must_use]
    pub const fn offline_cooldown_remaining_ms(&self) -> u64 {
        self.store.state().offline_runtime.cooldown_remaining_ms
    }

    /// What this device declares it can do.
    #[must_use]
    pub const fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// The simulated world, for the control API and the fault injector.
    #[must_use]
    pub const fn environment(&self) -> &Environment {
        &self.environment
    }

    /// Mutable access to the simulated world.
    ///
    /// This is the **environment**, not the pump: it can flood a tray or empty
    /// a reservoir, and it cannot deliver a dose that skipped the validator,
    /// because the only caller that moves water is the actuation path in
    /// M2-008.
    pub const fn environment_mut(&mut self) -> &mut Environment {
        &mut self.environment
    }

    /// How many cycles are waiting for a connection.
    #[must_use]
    pub fn buffered_cycles(&self) -> usize {
        self.telemetry_ring.len()
    }

    /// Marks the actuator as faulted or healthy.
    ///
    /// Fault state, not actuation: a faulted actuator is one the validator will
    /// refuse to run, so this can only ever make the device *less* permissive.
    pub const fn set_actuator_faulted(&mut self, faulted: bool) {
        self.actuator_faulted = faulted;
    }

    /// Whether the actuator reports a fault.
    #[must_use]
    pub const fn actuator_faulted(&self) -> bool {
        self.actuator_faulted
    }

    /// The faults currently injected.
    #[must_use]
    pub const fn faults(&self) -> &FaultSet {
        &self.faults
    }

    /// Enables a fault at runtime.
    pub fn enable_fault(&mut self, fault: crate::cli::Fault) {
        self.faults.enable(fault);
        self.apply_faults();
    }

    /// How much longer the `disconnect` fault should keep the device isolated.
    ///
    /// The device counts this down on its own monotonic clock rather than the
    /// driver holding a wall-clock timer, so an isolation lasts the same number
    /// of *virtual* seconds at any `--time-scale`.
    #[must_use]
    pub const fn isolation_remaining_ms(&self) -> u64 {
        self.isolation_remaining_ms
    }

    /// Whether the device is being held off the broker by an injected fault.
    #[must_use]
    pub const fn is_isolated_by_fault(&self) -> bool {
        self.isolation_remaining_ms > 0
    }

    /// Disables a fault at runtime.
    pub fn disable_fault(&mut self, name: &str) {
        let canonical = name.split_once(':').map_or(name, |(name, _)| name);
        if self.faults.disable(canonical) {
            if canonical == "disconnect" {
                self.isolation_remaining_ms = 0;
            }
            self.apply_faults();
        }
    }

    /// Pushes the enabled faults into the mechanisms that carry them.
    ///
    /// Re-applied whenever the set changes, so disabling a fault really undoes
    /// it rather than leaving the device stuck in the state it caused.
    fn apply_faults(&mut self) {
        let leak = self.faults.is_enabled("leak");
        self.environment.tank.set_leak(if leak {
            LeakState::Detected
        } else {
            LeakState::Clear
        });
        if self.faults.is_enabled("tank-empty") {
            self.environment.tank.set_percent(0.0);
        }
        self.pump_delivers = !self.faults.is_enabled("pump-no-delivery");
        self.pump
            .set_stuck_on(self.faults.is_enabled("pump-stuck-on"));
        if let Some(duration) = self.faults.disconnect_ms()
            && self.isolation_remaining_ms == 0
        {
            self.isolation_remaining_ms = duration;
            // Losing the broker is losing the connection, not losing the
            // device: sampling, the physical model, and buffering all continue
            // (protocol §5.12).
            self.on_disconnected();
        }
        if let Some(count) = self.faults.miss_wakes() {
            // Consumed at the next sleep: the device skips that many wakes and
            // says nothing at all about it, which is what makes the absence
            // unexplained from the Edge's side (SCEN-111).
            self.power.miss_wakes(count);
        }
        if !self.faults.is_enabled("stuck-sensor") {
            // Disabling really unfreezes. A fault that could not be undone
            // would make a scenario's later phases untestable — and a sensor
            // that stayed stuck after the fault was cleared would be a second,
            // undocumented fault.
            self.stuck_readings = None;
        }
    }

    /// Makes the pump run without moving water (`pump-no-delivery`).
    pub const fn set_pump_delivers(&mut self, delivers: bool) {
        self.pump_delivers = delivers;
    }

    /// The device identity.
    #[must_use]
    pub fn device_id(&self) -> &DeviceId {
        self.identity.device_id()
    }

    /// Milliseconds since this boot.
    #[must_use]
    pub const fn uptime_ms(&self) -> u64 {
        self.monotonic.elapsed_ms()
    }

    /// The configuration currently in force.
    #[must_use]
    pub const fn config(&self) -> &EffectiveConfig {
        &self.config
    }

    /// Whether the device's wall clock is currently trustworthy.
    ///
    /// `clock-unsync` forces this false whatever the Edge has said, which makes
    /// every water command refuse with `clock_unsynced` — the fault's whole
    /// purpose.
    #[must_use]
    pub fn clock_synced(&self) -> bool {
        !self.faults.is_enabled("clock-unsync")
            && self.time_sync.is_synced(self.monotonic.elapsed_ms())
    }

    /// Whether the device believes it is connected to the broker.
    #[must_use]
    pub const fn is_connected(&self) -> bool {
        self.connected
    }

    /// The subscriptions to establish on every connect.
    ///
    /// Exact topics from the contract, never a wildcard: `commands/+` would
    /// also match `commands/result`, which this device publishes.
    #[must_use]
    pub fn subscriptions(&self) -> [String; 8] {
        Topic::device_subscriptions(self.device_id())
    }

    /// Advances virtual time and returns whatever that made due.
    ///
    /// Takes the elapsed interval rather than reading a clock, so a test moves
    /// six hours in a microsecond and the whole schedule stays assertable.
    pub fn tick(&mut self, elapsed_ms: u64) -> Vec<Publication> {
        // `restart` reboots the device once, at the first opportunity after it
        // is enabled, then disables itself. A fault that restarted forever
        // would be a device that never runs, which is not a failure mode
        // anything needs to be tested against.
        if self.faults.is_enabled("restart") {
            tracing::warn!("restart: rebooting the device");
            self.disable_fault("restart");
            self.restart();
            return Vec::new();
        }
        self.monotonic.advance_ms(elapsed_ms);
        self.environment.step(elapsed_ms);
        // The plant does not sleep. Soil, reservoir, and pot keep evolving while
        // the device is off the air, which is what makes a missed wake visible
        // in the readings that follow it.
        if self.power.advance(elapsed_ms) {
            // One wake, one charge.
            self.credited_timer_wakes = self.credited_timer_wakes.saturating_add(1);
            self.environment.battery.drain_wake();
            tracing::debug!("woke from deep sleep");
        }
        if self.power.is_sleeping() {
            return Vec::new();
        }
        if self.isolation_remaining_ms > 0 {
            self.isolation_remaining_ms = self.isolation_remaining_ms.saturating_sub(elapsed_ms);
            if self.isolation_remaining_ms == 0 {
                tracing::info!("the injected isolation has elapsed; reconnecting");
                self.faults.disable("disconnect");
            }
        }
        self.advance_offline_runtime(elapsed_ms);
        let mut publications = self.step_pump(elapsed_ms);
        // Offline autonomy (M6-019). Isolation-only, opt-in, and bounded by the
        // shared evaluator; see `offline.rs` for why it is exactly one call.
        publications.extend(self.evaluate_offline_autonomy(elapsed_ms));

        // Sampling, buffering, and physical evolution continue whether or not
        // the broker is reachable: an isolated device is still a fully
        // functioning sensor node (protocol §5.12, ADR-015).
        if let Some(batch) = self.due_sample() {
            self.power.note_sampled();
            match self.telemetry_publication(batch.clone()) {
                Ok(p) if self.connected => publications.push(p),
                Ok(_) => {
                    self.telemetry_ring.push(batch);
                    if !self.power.is_battery() {
                        self.record_event(
                            EventTier::Telemetry,
                            EventKind::Unknown("telemetry.cycle".to_owned()),
                            EventDetail::Unknown,
                        );
                    }
                }
                Err(e) => tracing::error!(error = %e, "could not build a telemetry batch"),
            }
        }
        // An actuator change while isolated is not published and not queued:
        // the state itself is what matters, and the next connection republishes
        // whatever it is *then*, rather than a stale transition.
        if let Some(p) = self.actuator_publication_if_changed()
            && self.connected
        {
            publications.push(p);
        }

        if !self.connected {
            return publications;
        }
        // Results the edge has not acknowledged are republished on a timer, not
        // only on reconnect. The failure this covers is an edge that crashes
        // and restarts while the device's own socket never drops, so nothing
        // else would ever prompt a retry (protocol §5.14).
        publications.extend(self.retry_unacknowledged_results());
        let now = self.monotonic.elapsed_ms();
        let synced = self.clock_synced();
        // An unsynchronised device republishes its retained status at a bounded
        // rate: the status *is* the request for `edge.time` (protocol §5.12).
        // Adding a dedicated request topic would be a second way to say the
        // same thing.
        let due_after = if synced {
            self.config.heartbeat_interval_ms()
        } else {
            self.config
                .heartbeat_interval_ms()
                .min(UNSYNCED_STATUS_INTERVAL_MS)
        };
        if now.saturating_sub(self.last_status_ms) >= due_after || synced != self.last_reported_sync
        {
            match self.status_publication(DeviceStatusValue::Online, None) {
                Ok(p) => publications.push(p),
                Err(e) => tracing::error!(error = %e, "could not build the status heartbeat"),
            }
        }
        // A battery device goes back to sleep once its wake has done its work.
        // The announcement is published *before* the disconnect: the run loop
        // drops the socket only after this publication has gone out.
        let busy = self.actuator_active || self.pump.is_running();
        if self.power.should_sleep(busy) {
            if self.faults.is_enabled("sleep-without-announcing") {
                // Leave without a word. The will fires, the edge sees
                // `connection_lost`, and the absence is unexplained -- which is
                // exactly what SCEN-112 is about.
                tracing::warn!("sleep-without-announcing: leaving the broker silently");
                self.sleep_announced = false;
            } else {
                match self
                    .status_publication(DeviceStatusValue::Offline, Some(String::from("sleeping")))
                {
                    Ok(p) => publications.push(p),
                    Err(e) => {
                        tracing::error!(error = %e, "could not build the sleep announcement");
                    }
                }
                self.sleep_announced = true;
            }
            self.power.sleep();
            self.sleep_notice = true;
        }
        publications
    }

    /// Records the conservative RTC-checksum failure branch used by battery tests.
    pub const fn mark_rtc_checksum_failure(&mut self) {
        self.zero_credit_checksum_failures = self.zero_credit_checksum_failures.saturating_add(1);
    }

    /// Credited timer wake count.
    pub const fn credited_timer_wakes(&self) -> u64 {
        self.credited_timer_wakes
    }

    /// Zero-credit cold-reset count.
    pub const fn zero_credit_resets(&self) -> u64 {
        self.zero_credit_resets
    }

    /// Zero-credit checksum-failure count.
    pub const fn zero_credit_checksum_failures(&self) -> u64 {
        self.zero_credit_checksum_failures
    }

    /// Whether the device has announced its sleep and is waiting for the run
    /// loop to drop the socket. Consuming the flag means it is acted on once.
    pub const fn take_sleep_notice(&mut self) -> bool {
        let notice = self.sleep_notice;
        self.sleep_notice = false;
        notice
    }

    /// Whether the device is currently off the air.
    #[must_use]
    pub const fn is_sleeping(&self) -> bool {
        self.power.is_sleeping()
    }

    /// Whether the sleep now beginning was announced.
    ///
    /// An announced sleep leaves the broker **cleanly**, so no will fires and
    /// the retained `sleeping` status stays the last word on the device. An
    /// unannounced one drops the socket, the will fires, and the edge sees
    /// `connection_lost` -- which is the distinction SAFETY-021 turns on.
    #[must_use]
    pub const fn sleep_was_announced(&self) -> bool {
        self.sleep_announced
    }

    /// The wake cycle, for tests and the control API.
    #[must_use]
    pub const fn power_state(&self) -> &crate::power::PowerState {
        &self.power
    }

    /// Sets the simulated state of charge, for a test that needs a low battery
    /// now rather than in a fortnight.
    pub fn set_battery_percent(&mut self, percent: f64) {
        self.environment.battery.set_percent(percent);
    }

    /// The simulated state of charge.
    #[must_use]
    pub fn battery_percent(&self) -> f64 {
        self.environment.battery.true_percent()
    }

    /// Advances the persisted offline runtime state by observed time.
    ///
    /// **M2 advances it; M2 does not act on it.** The cooldown counts down and
    /// the budget window fills from monotonic milliseconds the device actually
    /// observed — never from a wall-clock difference across a reboot, which it
    /// cannot vouch for (SAFETY-015).
    ///
    /// The write is coalesced: persisting on every 100 ms tick would rewrite the
    /// state file ten times a second for a value that only matters across a
    /// restart. The in-memory value is always current; the file is brought up to
    /// date at least every `OFFLINE_RUNTIME_PERSIST_MS` of virtual time, which
    /// bounds how much observed time a crash can discard — and discarding
    /// observed time is conservative, since it leaves the cooldown *longer* and
    /// the budget window *shorter* than reality.
    /// A fresh identifier from the device's own generator.
    ///
    /// Uses the wall clock when it has one (UUIDv7, so ids sort by issue time)
    /// and a v4 otherwise. An isolated device must be able to name what it did
    /// without an Edge and without a calendar.
    pub fn next_local_uuid(&mut self) -> uuid::Uuid {
        let wall = self.wall_now();
        self.identity.next_uuid(wall)
    }

    /// The offline refusal most recently recorded.
    #[must_use]
    pub const fn last_offline_refusal(&self) -> Option<rhizo_policy::RefuseReason> {
        self.last_offline_refusal
    }

    /// Records the offline refusal now in force.
    pub const fn set_last_offline_refusal(&mut self, reason: Option<rhizo_policy::RefuseReason>) {
        self.last_offline_refusal = reason;
    }

    fn advance_offline_runtime(&mut self, elapsed_ms: u64) {
        if elapsed_ms == 0 {
            return;
        }
        self.unpersisted_runtime_ms = self.unpersisted_runtime_ms.saturating_add(elapsed_ms);
        let window_ms = self.policy_window_ms();
        if self.unpersisted_runtime_ms < OFFLINE_RUNTIME_PERSIST_MS {
            return;
        }
        let pending = std::mem::take(&mut self.unpersisted_runtime_ms);
        if let Err(e) = self.store.mutate(|state| {
            state.offline_runtime.advance(pending);
            if let Some(window_ms) = window_ms {
                state.offline_runtime.roll_window(window_ms);
            }
        }) {
            tracing::error!(error = %e, "could not persist the offline runtime state");
        }
    }

    /// The rolling window length of the first activated policy, if any.
    fn policy_window_ms(&self) -> Option<u64> {
        self.store
            .state()
            .policy_active
            .as_ref()
            .and_then(|stored| stored.payload.policies.first())
            .map(|policy| u64::from(policy.limits.window_ms))
    }

    /// Takes a sampling cycle if one is due.
    ///
    /// A leak state change forces a cycle immediately rather than waiting for
    /// the schedule: an hour-late leak notification is useless, and the leak
    /// input is a hard veto on actuation at both ends of the system.
    fn due_sample(&mut self) -> Option<TelemetryBatch> {
        let now = self.monotonic.elapsed_ms();
        let leak = self.environment.tank.leak();
        let leak_changed = leak != self.last_leak;
        // A reading taken before the peripherals have settled is not a reading
        // (ADR-018 section 5). A mains device is always settled.
        if !self.power.readings_usable() {
            return None;
        }
        // A battery device samples once per wake: the wake *is* the schedule.
        let scheduled = self.power.is_battery()
            || now.saturating_sub(self.last_sample_ms) >= self.config.telemetry_interval_ms();
        if !scheduled && !leak_changed {
            return None;
        }
        self.last_sample_ms = now;
        self.last_leak = leak;
        if self.capabilities.sensors().is_empty() {
            // An empty batch MUST NOT be published (protocol §5.2), and a
            // device configured with no sensors is a legitimate thing to run.
            return None;
        }
        let batch_id = self.identity.next_uuid(self.wall_now());
        let stuck = self.stuck_readings.clone();
        let invalid_soil_rate = self.faults.rate("invalid-soil");
        let batch = telemetry::sample_cycle(
            &self.capabilities,
            &mut self.environment,
            batch_id,
            |_, _| SensorHealth::Ok,
            |kind, _reading| {
                // `stuck-sensor`: hand back the *memorised* value, bit for bit.
                // A value that merely changed slowly would still look alive to
                // stuck-sensor detection, which is the thing under test.
                stuck.as_ref().and_then(|m| m.get(kind.as_str())).copied()
            },
        );
        let mut batch = batch;
        // The `stale-*` faults withhold one kind while every other stream keeps
        // reporting, which is the shape SAFETY-005 and SAFETY-017 care about: a
        // device that is plainly present with one input quietly ageing out.
        // Withholding everything would be the offline fault instead.
        for (fault, kind) in [
            ("stale-soil", MeasurementKind::SoilMoisture),
            ("stale-tank", MeasurementKind::TankLevel),
            ("stale-leak", MeasurementKind::LeakState),
            ("stale-weight", MeasurementKind::PotWeight),
        ] {
            if self.faults.is_enabled(fault) {
                batch.samples.retain(|sample| sample.kind != kind);
            }
        }
        if invalid_soil_rate > 0.0 {
            self.spoil_soil_readings(&mut batch);
        }
        // Per-kind ages, for the staleness checks M6-019 will make. Kept in
        // RAM only: a reading that survived a reboot would carry an age the
        // device cannot vouch for, and an unknown-age reading must count as
        // missing rather than as fresh (SAFETY-017).
        let taken_at = self.monotonic.elapsed_ms();
        for sample in &batch.samples {
            self.recent_samples
                .record(&sample.kind, sample.value, sample.quality, taken_at);
        }
        if self.faults.is_enabled("stuck-sensor") && self.stuck_readings.is_none() {
            // Freeze on the first cycle after the fault is enabled, so the
            // frozen value is a real reading rather than an invented one.
            self.stuck_readings = Some(
                batch
                    .samples
                    .iter()
                    .filter_map(|s| s.value.map(|v| (s.kind.as_str().to_owned(), v)))
                    .collect(),
            );
        }
        Some(batch)
    }

    /// Spoils soil-moisture samples for `invalid-soil`.
    ///
    /// Two shapes, alternating on the deterministic draw, because the edge has
    /// to handle both: an out-of-range but finite value (protocol §10 stores the
    /// message and nulls that field), and a failed read (`value: null` with
    /// `quality: "fault"`). Emitting `NaN` is **not** one of them — §4 forbids
    /// putting a non-finite number on the wire at all, so a real faulty sensor
    /// publishes null instead.
    fn spoil_soil_readings(&mut self, batch: &mut TelemetryBatch) {
        use rhizo_mqtt_contract::payload::{MeasurementValue, Quality};
        for sample in &mut batch.samples {
            if sample.kind != MeasurementKind::SoilMoisture {
                continue;
            }
            if !self.faults.fires("invalid-soil", &mut self.environment.rng) {
                continue;
            }
            if self.environment.rng.chance(0.5) {
                // Out of range for `soil_moisture`, and finite.
                sample.value = Some(MeasurementValue::Scalar(150.0));
                sample.quality = Quality::Suspect;
            } else {
                sample.value = None;
                sample.quality = Quality::Fault;
            }
        }
    }

    /// Seals a telemetry batch. Never retained — the flag comes from the topic.
    fn telemetry_publication(
        &mut self,
        batch: TelemetryBatch,
    ) -> Result<Publication, serde_json::Error> {
        let topic = Topic::Telemetry(self.device_id().clone());
        let wall = self.wall_now();
        let synced = wall.is_some();
        self.identity.seal(topic, batch, wall, synced)
    }

    /// Publishes actuator state, but only when it has changed.
    ///
    /// Actuator state is *state*, not a measurement, which is why it is not in
    /// the batch and why it is published on change rather than periodically
    /// (protocol §5.3).
    fn actuator_publication_if_changed(&mut self) -> Option<Publication> {
        let actuator = self.capabilities.primary_actuator()?;
        let current = ActuatorState {
            actuator_id: actuator.actuator_id.clone(),
            kind: actuator.kind,
            active: self.actuator_active,
            last_run_ms: self.actuator_last_run_ms,
            delivered_today_ml: self.store.state().delivered_today_ml,
            faulted: self.actuator_faulted,
        };
        if self.last_actuator.as_ref() == Some(&current) {
            return None;
        }
        self.last_actuator = Some(current.clone());
        let topic = Topic::Actuator(self.device_id().clone());
        let wall = self.wall_now();
        let synced = wall.is_some();
        match self.identity.seal(topic, current, wall, synced) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::error!(error = %e, "could not build the actuator state");
                None
            }
        }
    }

    /// The device's wall time, present only while synchronised.
    pub(crate) fn wall_now(&self) -> Option<rhizo_mqtt_contract::UtcMillis> {
        if !self.clock_synced() {
            return None;
        }
        let now = self.time_sync.synced_now_ms(self.monotonic.elapsed_ms())?;
        // `clock-skew` offsets the device's idea of the time. It is applied
        // here, at the one place wall time is read, so the skew reaches the
        // envelope *and* the expiry check together — a device whose published
        // timestamps disagreed with the times it validated against would be a
        // fault nobody has.
        Some(rhizo_mqtt_contract::UtcMillis(
            now.0.saturating_add(self.faults.clock_skew_ms()),
        ))
    }

    /// Handles a message on one of the device's subscriptions.
    pub fn on_message(&mut self, topic: &Topic, payload: &[u8]) -> Vec<Publication> {
        match topic {
            Topic::Config(_) => self.on_config(payload),
            Topic::Time(_) => self.on_time(payload),
            // Policy handling is M2-016; command handling is M2-008. Until then
            // the device receives and ignores them, which is the conservative
            // behaviour: an unimplemented command is not an executed one.
            Topic::CommandWater(_) => self.on_water_command(payload),
            Topic::CommandTare(_) => self.on_tare_command(payload),
            Topic::CommandCalibrate(_) => self.on_calibrate_command(payload),
            Topic::Policy(_) => self.on_policy(payload),
            Topic::EventsAck(_) => self.on_event_ack(payload),
            Topic::CommandResultAck(_) => self.on_command_result_ack(payload),
            // Never subscribed to: the device publishes all of these. Since the
            // subscriptions became exact topics, the broker cannot deliver one
            // here at all — refusing again is belt and braces, so no future call
            // site can route one in by mistake.
            Topic::CommandResult(_)
            | Topic::Telemetry(_)
            | Topic::Actuator(_)
            | Topic::Events(_)
            | Topic::Status(_) => Vec::new(),
        }
    }

    /// Advances the pump and turns a completed run into water and a result.
    ///
    /// The delivery happens **after** the run finishes rather than at accept
    /// time, so a restart mid-dose really does leave the outcome unknown — the
    /// property `--fault restart-mid-dose` exists to exercise.
    fn step_pump(&mut self, elapsed_ms: u64) -> Vec<Publication> {
        let (command_id, duration_ms, effective_ml, status) = match self.pump.step(elapsed_ms) {
            PumpStep::Idle | PumpStep::Running => return Vec::new(),
            PumpStep::Finished {
                command_id,
                duration_ms,
                effective_ml,
            } => (
                command_id,
                duration_ms,
                effective_ml,
                CommandStatus::Completed,
            ),
            PumpStep::GuardTripped {
                command_id,
                duration_ms,
                effective_ml,
            } => {
                tracing::error!(
                    command_id = %command_id,
                    duration_ms,
                    "the pump failed to de-energise; the independent run guard stopped it"
                );
                // `failed`, not `completed`: the hardware misbehaved, and the
                // volume is credited conservatively (protocol §5.10).
                (command_id, duration_ms, effective_ml, CommandStatus::Failed)
            }
        };
        self.actuator_active = false;
        self.actuator_last_run_ms = Some(duration_ms);
        // The result reports what the *edge asked for*, not what the gate
        // authorised: `requested_ml` plus `clamped` is how the edge learns that
        // a hard limit reduced its request. Reporting the clamped figure as the
        // request would hide the clamp entirely.
        let requested_ml = self
            .store
            .state()
            .in_flight_dose
            .as_ref()
            .map_or(effective_ml, |dose| dose.requested_ml);

        let delivered = if self.pump_delivers {
            self.environment.deliver_water(f64::from(effective_ml))
        } else {
            // `pump-no-delivery`: the pump runs, the reservoir does not fall,
            // and the scale does not move. That divergence is the signature the
            // weight-based no-delivery detection has to find.
            self.environment.deliver_nothing()
        };
        let credited = match status {
            CommandStatus::Completed => delivered.delivered_ml as f32,
            // A failed run delivered an unknown amount; credit what was
            // authorised rather than what was observed.
            _ => effective_ml,
        };
        let wall = self.wall_now();
        if let Err(e) = self.store.mutate(|state| {
            state.credit_delivery(credited, wall);
            state.in_flight_dose = None;
        }) {
            tracing::error!(error = %e, "could not record the delivery");
        }
        let result = CommandResult {
            command_id,
            status,
            requested_ml,
            delivered_ml: match status {
                CommandStatus::Completed => Some(delivered.delivered_ml as f32),
                _ => None,
            },
            duration_ms: Some(duration_ms),
            clamped: self.pending_clamped,
            reason: None,
            delivered_today_ml: self.store.state().delivered_today_ml,
            origin: self.pending_origin,
            detail: self.pending_detail.take(),
        };
        tracing::info!(
            command_id = %command_id,
            ?status,
            delivered_ml = delivered.delivered_ml,
            duration_ms,
            "dose finished"
        );
        self.record_and_queue(result)
    }

    /// Applies a retained `device.config`.
    fn on_config(&mut self, payload: &[u8]) -> Vec<Publication> {
        let envelope = match self.decode::<DeviceConfig>(payload, "device.config") {
            Some(e) => e,
            None => return Vec::new(),
        };
        match self.config.consider(&envelope.data) {
            ConfigOutcome::Applied { version } => {
                tracing::info!(config_version = version, "configuration applied");
                // The edge owns the wake cycle from here (M5-019). A
                // configuration changes what the *next* cycle does; it cannot
                // wake a sleeping device or put an awake one to sleep mid-cycle,
                // because a retained message that could do either would be a way
                // to strand a device.
                //
                // An **absent** `power` block declares nothing and changes
                // nothing — it is what every configuration written before
                // ADR-018 carries, and treating it as an explicit `always_on`
                // would let a stale retained config silently retire a battery
                // device's sleep. That is not hypothetical: it is exactly what a
                // broker holding a pre-ADR-018 retained `device.config` does on
                // the device's very first subscribe.
                if let Some(power) = envelope.data.power {
                    self.power.reconfigure(
                        power.mode,
                        power.wake_interval_seconds,
                        power.sensor_warmup_ms,
                        power.awake_budget_seconds,
                    );
                }
                // Persisted immediately: a config the device is running but has
                // not stored would be silently forgotten by the next restart,
                // and the edge would see no drift because the device would
                // accept the retained republication again.
                if let Err(e) = self
                    .store
                    .mutate(|state| state.applied_config_version = Some(version))
                {
                    tracing::error!(error = %e, "could not persist the applied config version");
                }
                match self.status_publication(DeviceStatusValue::Online, None) {
                    Ok(p) => vec![p],
                    Err(e) => {
                        tracing::error!(error = %e, "could not build the status after a config change");
                        Vec::new()
                    }
                }
            }
            ConfigOutcome::Rejected { error } => {
                // The previous configuration stays in force and the device keeps
                // reporting the version it is actually running, so the edge sees
                // drift rather than a silent acceptance.
                tracing::warn!(
                    ?error,
                    offered = envelope.data.config_version,
                    applied = ?self.config.applied_version,
                    "configuration rejected; the previous one is retained"
                );
                Vec::new()
            }
            ConfigOutcome::IgnoredVersion { offered, applied } => {
                tracing::debug!(
                    offered,
                    applied,
                    "configuration ignored: not newer than the applied version"
                );
                Vec::new()
            }
        }
    }

    /// Applies an `edge.time`.
    fn on_time(&mut self, payload: &[u8]) -> Vec<Publication> {
        let envelope = match self.decode::<EdgeTime>(payload, "edge.time") {
            Some(e) => e,
            None => return Vec::new(),
        };
        let now = self.monotonic.elapsed_ms();
        let was_synced = self.clock_synced();
        if !self.time_sync.apply(envelope.data, now) {
            tracing::debug!(
                offered_ms = envelope.data.edge_time_ms.0,
                last_applied_ms = ?self.time_sync.last_applied().map(|t| t.0),
                "edge.time ignored: not strictly newer than the last applied"
            );
            return Vec::new();
        }
        if was_synced {
            tracing::debug!(
                edge_time_ms = envelope.data.edge_time_ms.0,
                "wall clock synchronisation refreshed from the edge"
            );
        } else {
            tracing::info!(
                edge_time_ms = envelope.data.edge_time_ms.0,
                "wall clock synchronised from the edge"
            );
        }
        // Announce the transition rather than waiting for the next heartbeat:
        // `clock_synced` is what tells the edge its commands will be accepted.
        if was_synced {
            return Vec::new();
        }
        match self.status_publication(DeviceStatusValue::Online, None) {
            Ok(p) => vec![p],
            Err(e) => {
                tracing::error!(error = %e, "could not build the status after synchronising");
                Vec::new()
            }
        }
    }

    /// Decodes an inbound envelope, logging the typed reason on failure.
    ///
    /// A message whose *identity* is inconsistent is untrustworthy in its
    /// entirety and is rejected whole (protocol §10).
    /// Decodes an inbound command envelope.
    pub(crate) fn decode_command<T: serde::de::DeserializeOwned>(
        &self,
        payload: &[u8],
        kind: &'static str,
    ) -> Option<Envelope<T>> {
        self.decode(payload, kind)
    }

    /// Seals a `command.result`. Never retained — the flag comes from the topic.
    pub(crate) fn seal_result(
        &mut self,
        topic: Topic,
        result: CommandResult,
    ) -> Result<Publication, serde_json::Error> {
        let wall = self.wall_now();
        let synced = wall.is_some();
        self.identity.seal(topic, result, wall, synced)
    }

    pub(crate) fn decode<T: serde::de::DeserializeOwned>(
        &self,
        payload: &[u8],
        kind: &'static str,
    ) -> Option<Envelope<T>> {
        let envelope = match Envelope::<T>::from_json(payload) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(kind, reason = e.metric_reason(), "inbound message rejected");
                return None;
            }
        };
        if let Err(e) = envelope.check_identity(self.device_id()) {
            tracing::warn!(kind, reason = e.metric_reason(), "inbound message rejected");
            return None;
        }
        Some(envelope)
    }

    /// The Last Will and Testament, built **before** connecting.
    ///
    /// Setting a will after connecting silently does nothing, and the omission
    /// is invisible until a device dies in a test.
    ///
    /// # Errors
    ///
    /// Returns a serialisation failure, which cannot happen for this payload.
    pub fn will(&mut self) -> Result<Publication, serde_json::Error> {
        let topic = Topic::Status(self.device_id().clone());
        let data = DeviceStatus {
            status: DeviceStatusValue::Offline,
            reason: Some(String::from("connection_lost")),
            ..self.status_skeleton()
        };
        self.identity.seal_will(topic, data)
    }

    /// Handles a successful connection: retained `status: online`.
    ///
    /// # Errors
    ///
    /// Returns a serialisation failure, which cannot happen for this payload.
    pub fn on_connected(&mut self) -> Result<Vec<Publication>, serde_json::Error> {
        self.connected = true;
        let isolated_ms = self.isolation.on_connected(self.monotonic.elapsed_ms());
        if isolated_ms > 0 {
            tracing::info!(isolated_ms, "reconnected after running alone");
        }
        let mut publications = vec![self.status_publication(DeviceStatusValue::Online, None)?];
        // Flush the bounded ring, oldest first. A device MUST NOT flush a
        // backlog beyond it (protocol §8 step 5) — the ring is the bound.
        for batch in self.telemetry_ring.drain() {
            publications.push(self.telemetry_publication(batch)?);
        }
        // Republish any result that outlived a disconnect or a reboot. A result
        // is ledger data: the edge's view of what happened to a plant depends on
        // it arriving eventually, not promptly.
        publications.extend(self.flush_results());
        // Replay buffered history, in `device_seq` order, ending with
        // `complete: true` (protocol §8 step 7).
        publications.extend(self.replay_publications());
        Ok(publications)
    }

    /// Handles a lost connection.
    ///
    /// Publishes nothing: the broker delivers the will. A device that could
    /// publish its own `offline` on an *unclean* disconnect would not be a
    /// device — that is precisely the case the will exists to cover.
    pub fn on_disconnected(&mut self) {
        self.isolation.on_disconnected(self.monotonic.elapsed_ms());
        self.connected = false;
    }

    /// The device's own view of its connectivity (protocol §5.5).
    ///
    /// While isolated, `isolated_ms` is how long the current isolation has run.
    /// While connected it is how long the **most recent** isolation lasted —
    /// which is the only way the edge can ever learn it, since a device cannot
    /// publish while it is isolated. `mode` disambiguates the two, and the
    /// value is stable rather than reported once, so a lost status message does
    /// not lose the fact that a plant ran alone for six hours.
    #[must_use]
    pub fn connectivity(&self) -> Connectivity {
        self.isolation.connectivity(self.monotonic.elapsed_ms())
    }

    /// Applies an `event.ack` (protocol §5.13).
    ///
    /// Deletion is the *last* step and happens in one persisted mutation, so a
    /// crash between "decided to delete" and "wrote the file" leaves the events
    /// still buffered. Replaying history the edge already has costs bandwidth;
    /// deleting history it does not have loses it for ever.
    fn on_event_ack(&mut self, payload: &[u8]) -> Vec<Publication> {
        let Some(envelope) = self.decode::<EventAck>(payload, "event.ack") else {
            return Vec::new();
        };
        let ack = envelope.data;

        // An acknowledgement names the boot whose replay it covers. A delayed
        // one from a previous run says nothing about the history this run is
        // holding — sequences continue across boots, so a stale acknowledgement
        // would silently cover events buffered since. Refuse it.
        if ack.boot_id != self.identity.boot_id() {
            tracing::warn!(
                acked_boot = %ack.boot_id,
                this_boot = %self.identity.boot_id(),
                "ignoring an acknowledgement for another boot"
            );
            return Vec::new();
        }

        let outcome = match self
            .store
            .mutate(|state| state.offline_events.acknowledge(ack.through_device_seq))
        {
            Ok(outcome) => outcome,
            Err(e) => {
                tracing::error!(error = %e, "could not persist the acknowledgement; history is kept");
                return Vec::new();
            }
        };
        match outcome {
            AckOutcome::Applied {
                through_seq,
                removed,
            } => tracing::info!(through_seq, removed, "replay acknowledged"),
            AckOutcome::NotNewer { through_seq } => tracing::debug!(
                through_seq,
                applied = ?self.store.state().offline_events.pending_ack_through_seq,
                "acknowledgement is not newer; ignored"
            ),
            AckOutcome::BeyondKnown {
                through_seq,
                highest,
            } => tracing::error!(
                through_seq,
                highest,
                "refusing an acknowledgement beyond any sequence this device issued; \
                 nothing was deleted"
            ),
        }
        self.last_ack_outcome = Some(outcome);
        Vec::new()
    }

    /// This boot's `boot_id`.
    ///
    /// An acknowledgement names the boot it covers, so a caller has to be able
    /// to see which boot is current.
    #[must_use]
    pub const fn boot_id(&self) -> BootId {
        self.identity.boot_id()
    }

    /// The outcome of the most recent `event.ack`, for tests and diagnostics.
    #[must_use]
    pub const fn last_ack_outcome(&self) -> Option<AckOutcome> {
        self.last_ack_outcome
    }

    /// The sequence acknowledged through, if any.
    #[must_use]
    pub const fn acknowledged_through(&self) -> Option<u64> {
        self.store.state().offline_events.pending_ack_through_seq
    }

    /// Buffers a device event, giving it its one and only `event_id`.
    ///
    /// The id is drawn **here**, at buffering time, and every replay reuses it.
    /// Generating it at publish time instead would give the edge a different id
    /// for the same event on each replay, defeating deduplication and creating
    /// duplicate watering history — SAFETY-016's central failure.
    ///
    /// M2 produces the events it genuinely has: policy activation, and the
    /// lockout a persistent-state fault imposes. Autonomous outcomes arrive with
    /// the evaluator in M6-019 and go through this same method.
    pub fn record_event(
        &mut self,
        tier: EventTier,
        kind: EventKind,
        detail: EventDetail,
    ) -> Buffered {
        let wall = self.wall_now();
        let event_id = EventId::from_uuid(self.identity.next_uuid(wall));
        let monotonic_ms = self.monotonic.elapsed_ms();
        let outcome = self.store.mutate(|state| {
            state
                .offline_events
                .push(event_id, tier, kind, monotonic_ms, wall, detail)
        });
        match outcome {
            Ok(buffered) => {
                if let Buffered::Evicted {
                    lost_seq,
                    lost_tier,
                    ..
                } = buffered
                {
                    tracing::warn!(
                        lost_seq,
                        ?lost_tier,
                        "buffered history evicted; the loss is recorded as a gap"
                    );
                }
                buffered
            }
            Err(e) => {
                tracing::error!(error = %e, "could not persist a buffered event");
                Buffered::Stored { device_seq: 0 }
            }
        }
    }

    /// The highest `device_seq` this device has ever allocated.
    ///
    /// The ceiling an acknowledgement may not exceed (protocol §5.13).
    #[must_use]
    pub const fn highest_allocated_seq(&self) -> Option<u64> {
        self.store.state().offline_events.highest_allocated_seq()
    }

    /// The ids a replay would carry, in order.
    ///
    /// These are fixed at buffering time and must be identical across replays;
    /// that is what lets the edge deduplicate (SAFETY-016).
    #[must_use]
    pub fn buffered_event_ids(&self) -> Vec<EventId> {
        self.store
            .state()
            .offline_events
            .replay_events()
            .iter()
            .map(|e| e.event_id)
            .collect()
    }

    /// How many events are waiting to be replayed.
    #[must_use]
    pub fn buffered_events(&self) -> usize {
        self.store.state().offline_events.replay_events().len()
    }

    /// Discards history the edge has acknowledged.
    ///
    /// The same path `event.ack` takes, minus the wire decode, so a test and the
    /// protocol cannot diverge about what an acknowledgement does.
    pub fn acknowledge_events(&mut self, through_seq: u64) -> AckOutcome {
        let outcome = self
            .store
            .mutate(|state| state.offline_events.acknowledge(through_seq))
            .unwrap_or(AckOutcome::NotNewer { through_seq });
        self.last_ack_outcome = Some(outcome);
        outcome
    }

    /// Builds the replay publications for a reconnection.
    ///
    /// Always at least one batch, and the last always carries `complete: true`:
    /// the edge holds every plant on a reconnecting device in `Uncertain` until
    /// it sees that flag, so a device with nothing to say still has to say so
    /// (protocol §5.4, SAFETY-016).
    fn replay_publications(&mut self) -> Vec<Publication> {
        // Seal any still-growing gap first: once a marker is sent its range can
        // never change again, because the edge deduplicates on `event_id` and
        // would keep the first version for ever.
        if let Err(e) = self.store.mutate(|state| state.offline_events.seal_gap()) {
            tracing::error!(error = %e, "could not seal the pending history gap");
        }
        let batches = self
            .store
            .state()
            .offline_events
            .replay_batches(crate::buffer::REPLAY_BATCH);
        let topic = Topic::Events(self.device_id().clone());
        let mut publications = Vec::new();
        for batch in batches {
            let wall = self.wall_now();
            let synced = wall.is_some();
            match self.identity.seal(topic.clone(), batch, wall, synced) {
                Ok(publication) => publications.push(publication),
                Err(e) => tracing::error!(error = %e, "could not encode a replay batch"),
            }
        }
        publications
    }

    /// Per-kind last readings, with their monotonic ages.
    #[must_use]
    pub const fn recent_samples(&self) -> &RecentSamples {
        &self.recent_samples
    }

    /// The leak reading, or `None` when this device has no leak sensor.
    ///
    /// `None` rather than `Clear`: a device with no leak sensor has no evidence
    /// the tray is dry, and the difference is the whole of SAFETY-012.
    #[must_use]
    pub fn leak_reading(&self) -> Option<LeakState> {
        self.samples_kind(&MeasurementKind::LeakState)
            .then(|| self.environment.tank.leak())
    }

    /// The reservoir level, or `None` when this device has no level sensor.
    #[must_use]
    pub fn tank_reading(&self) -> Option<f32> {
        self.samples_kind(&MeasurementKind::TankLevel)
            .then(|| self.environment.tank.true_percent() as f32)
    }

    /// Whether the actuator reports healthy, or `None` when there is none.
    #[must_use]
    pub fn pump_health(&self) -> Option<bool> {
        self.capabilities
            .primary_actuator()
            .map(|_| !self.actuator_faulted && self.config.pump.enabled)
    }

    /// Whether the device believes it is alone (connectivity mode C).
    #[must_use]
    pub const fn is_isolated(&self) -> bool {
        self.isolation.is_isolated()
    }

    /// Handles a deliberate stop: `status: offline` with `reason: "shutdown"`,
    /// so an operator can tell "I stopped it" from "it died".
    ///
    /// # Errors
    ///
    /// Returns a serialisation failure, which cannot happen for this payload.
    pub fn on_shutdown(&mut self) -> Result<Vec<Publication>, serde_json::Error> {
        let publication =
            self.status_publication(DeviceStatusValue::Offline, Some(String::from("shutdown")))?;
        self.connected = false;
        Ok(vec![publication])
    }

    /// Builds a `device.status` publication and records that it happened.
    pub(crate) fn status_publication(
        &mut self,
        status: DeviceStatusValue,
        reason: Option<String>,
    ) -> Result<Publication, serde_json::Error> {
        let topic = Topic::Status(self.device_id().clone());
        let data = DeviceStatus {
            status,
            reason,
            ..self.status_skeleton()
        };
        let now = self.monotonic.elapsed_ms();
        // Use the public trust decision, not the raw synchroniser state.  The
        // `clock-unsync` fault deliberately makes an otherwise fresh clock
        // untrustworthy, and the retained status is what lets the Edge enforce
        // that fact before it sends a command.
        let synced = self.clock_synced();
        // `device_time_ms` is present only while synchronised, which is also
        // what makes `message_id` a UUIDv7 rather than a v4 (protocol §4).
        let wall = synced.then(|| self.time_sync.synced_now_ms(now)).flatten();
        self.last_status_ms = now;
        self.last_reported_sync = synced;
        self.identity.seal(topic, data, wall, synced)
    }

    /// The status fields every publication shares.
    ///
    /// `limits` reports the compile-time hard limits for observability.
    /// **Reporting is one-way**: no message can change them (SAFETY-007), which
    /// is why they are read straight from the contract crate's constants rather
    /// than from any field the device holds.
    fn status_skeleton(&self) -> DeviceStatus {
        DeviceStatus {
            boot_generation: self.store.state().boot_count,
            status: DeviceStatusValue::Online,
            reason: None,
            firmware_version: Some(String::from(FIRMWARE_VERSION)),
            protocol_version: Some(PROTOCOL_VERSION),
            applied_config_version: self.config.applied_version,
            uptime_ms: Some(self.monotonic.elapsed_ms()),
            free_heap_bytes: Some(SIMULATED_FREE_HEAP_BYTES),
            rssi_dbm: Some(SIMULATED_RSSI_DBM),
            applied_policy_versions: self.store.state().applied_policy_versions.clone(),
            connectivity: Some(self.connectivity()),
            // A mains device declares nothing at all, which is byte-for-byte
            // what every pre-ADR-018 device published.
            power: self
                .power
                .status(Some(self.environment.battery.sample_millivolts()))
                .map(Box::new),
            capabilities: self.capabilities.declaration(|_| (true, 0)),
            limits: Some(ReportedLimits {
                max_run_seconds: FIRMWARE_MAX_RUN_SECONDS,
                max_ml_per_run: FIRMWARE_MAX_ML_PER_RUN,
                max_daily_ml: FIRMWARE_MAX_DAILY_ML,
            }),
        }
    }
}

/// Convenience for tests and for the driver: the typed decode error a payload
/// produced, without needing the payload type.
#[must_use]
pub fn decode_reason(error: &DecodeError) -> &'static str {
    error.metric_reason()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Fault;
    use crate::testutil::cli;
    use rhizo_mqtt_contract::payload::{PumpConfig, SensorConfig, TankConfig};
    use rhizo_mqtt_contract::{MessageId, UtcMillis};
    use uuid::Uuid;

    fn decode(p: &Publication) -> Envelope<DeviceStatus> {
        let decoded = Envelope::<DeviceStatus>::from_json(p.payload.as_bytes()).unwrap();
        decoded.check_topic(&p.topic).unwrap();
        decoded
    }

    /// A tick now yields telemetry and actuator state as well as status, so a
    /// test about the status schedule has to look only at status.
    fn statuses(published: &[Publication]) -> Vec<Envelope<DeviceStatus>> {
        published
            .iter()
            .filter(|p| matches!(p.topic, Topic::Status(_)))
            .map(decode)
            .collect()
    }

    fn status_count(published: &[Publication]) -> usize {
        statuses(published).len()
    }

    fn telemetry(published: &[Publication]) -> Vec<Envelope<TelemetryBatch>> {
        published
            .iter()
            .filter(|p| matches!(p.topic, Topic::Telemetry(_)))
            .map(|p| {
                let decoded = Envelope::<TelemetryBatch>::from_json(p.payload.as_bytes()).unwrap();
                decoded.check_topic(&p.topic).unwrap();
                decoded
            })
            .collect()
    }

    fn connected(args: &[&str]) -> Device {
        let mut device = Device::new(&cli(args));
        device.on_connected().unwrap();
        device
    }

    fn config_payload(device_id: &str, version: u32, interval: u32) -> Vec<u8> {
        envelope_payload(
            device_id,
            "device.config",
            serde_json::to_value(DeviceConfig {
                config_version: version,
                telemetry_interval_seconds: interval,
                pump: PumpConfig {
                    ml_per_second: 8.2,
                    enabled: true,
                },
                tank: TankConfig { min_percent: 15.0 },
                sensors: SensorConfig::default(),
                power: None,
            })
            .unwrap(),
        )
    }

    fn time_payload(device_id: &str, edge_time_ms: i64) -> Vec<u8> {
        envelope_payload(
            device_id,
            "edge.time",
            serde_json::json!({ "edge_time_ms": edge_time_ms }),
        )
    }

    fn envelope_payload(device_id: &str, kind: &str, data: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "v": 1,
            "kind": kind,
            "message_id": MessageId::from_uuid(Uuid::nil()),
            "device_id": device_id,
            "data": data,
        }))
        .unwrap()
    }

    fn config_topic() -> Topic {
        Topic::Config(DeviceId::parse("plant-node-01").unwrap())
    }

    fn time_topic() -> Topic {
        Topic::Time(DeviceId::parse("plant-node-01").unwrap())
    }

    // ---------------------------------------------------------- connection

    #[test]
    fn connecting_publishes_retained_status_online() {
        let mut device = Device::new(&cli(&[]));
        let published = device.on_connected().unwrap();
        // Status, then the replay batch every reconnection ends with
        // (protocol §8 step 7): the edge holds the plant in `Uncertain` until
        // it sees `complete: true`, so a device with nothing buffered still
        // sends one empty complete batch.
        assert_eq!(statuses(&published).len(), 1);
        let p = published
            .iter()
            .find(|p| matches!(p.topic, Topic::Status(_)))
            .unwrap();
        assert!(p.retain, "status is retained (protocol §3)");
        assert_eq!(p.topic_string(), "rhizo/v1/devices/plant-node-01/status");
        let status = decode(p);
        assert_eq!(status.data.status, DeviceStatusValue::Online);
        assert_eq!(
            status.data.boot_generation,
            device.store().state().boot_count
        );
        assert!(status.data.boot_generation > 0);
        assert!(device.is_connected());
    }

    #[test]
    fn the_will_says_connection_lost_and_a_clean_stop_says_shutdown() {
        let mut device = Device::new(&cli(&[]));
        let will = decode(&device.will().unwrap());
        assert_eq!(will.data.status, DeviceStatusValue::Offline);
        assert_eq!(will.data.reason.as_deref(), Some("connection_lost"));

        device.on_connected().unwrap();
        let stop = device.on_shutdown().unwrap();
        let stop = decode(&stop[0]);
        assert_eq!(stop.data.status, DeviceStatusValue::Offline);
        assert_eq!(
            stop.data.reason.as_deref(),
            Some("shutdown"),
            "a deliberate stop must be distinguishable from a death"
        );
        assert!(!device.is_connected());
    }

    #[test]
    fn an_unclean_disconnect_publishes_nothing_because_the_broker_does() {
        let mut device = connected(&[]);
        device.on_disconnected();
        assert!(!device.is_connected());
        assert!(
            status_count(&device.tick(10_000_000)) == 0,
            "a disconnected device publishes nothing; it buffers instead"
        );
    }

    #[test]
    fn status_reports_the_compile_time_limits_verbatim() {
        let mut device = Device::new(&cli(&[]));
        let limits = decode(&device.on_connected().unwrap()[0])
            .data
            .limits
            .unwrap();
        assert_eq!(limits.max_run_seconds, FIRMWARE_MAX_RUN_SECONDS);
        assert_eq!(limits.max_ml_per_run, FIRMWARE_MAX_ML_PER_RUN);
        assert_eq!(limits.max_daily_ml, FIRMWARE_MAX_DAILY_ML);
    }

    #[test]
    fn subscriptions_are_the_normative_exact_filters_and_exclude_results() {
        let device = Device::new(&cli(&[]));
        assert_eq!(
            device.subscriptions(),
            [
                "rhizo/v1/devices/plant-node-01/config",
                "rhizo/v1/devices/plant-node-01/policy",
                "rhizo/v1/devices/plant-node-01/time",
                "rhizo/v1/devices/plant-node-01/commands/water",
                "rhizo/v1/devices/plant-node-01/commands/tare",
                "rhizo/v1/devices/plant-node-01/commands/calibrate",
                "rhizo/v1/devices/plant-node-01/commands/result/ack",
                "rhizo/v1/devices/plant-node-01/events/ack",
            ]
            .map(String::from)
        );
        assert!(
            !device
                .subscriptions()
                .contains(&"rhizo/v1/devices/plant-node-01/commands/result".to_owned()),
            "the result acknowledgement must not drag in the result topic itself"
        );
    }

    #[test]
    fn uptime_advances_with_the_monotonic_clock() {
        let mut device = Device::new(&cli(&[]));
        device.tick(90_000);
        let status = decode(&device.on_connected().unwrap()[0]);
        assert_eq!(status.data.uptime_ms, Some(90_000));
    }

    // ------------------------------------------------------------- status

    #[test]
    fn the_heartbeat_falls_on_five_sampling_intervals() {
        let mut device = connected(&["--telemetry-interval", "10"]);
        // Synchronise first, so the 60 s unsynchronised rate does not drive the
        // schedule and mask the heartbeat under test.
        device.on_message(&time_topic(), &time_payload("plant-node-01", 1_000));
        let heartbeat = device.config().heartbeat_interval_ms();
        assert_eq!(heartbeat, 50_000);

        let mut published = 0;
        for _ in 0..49 {
            published += status_count(&device.tick(1_000));
        }
        assert_eq!(published, 0, "nothing is due before the interval elapses");
        assert_eq!(
            status_count(&device.tick(1_000)),
            1,
            "the heartbeat is due at 5x"
        );
        for _ in 0..49 {
            published += status_count(&device.tick(1_000));
        }
        assert_eq!(published, 0);
        assert_eq!(status_count(&device.tick(1_000)), 1, "and repeats");
    }

    #[test]
    fn an_unsynchronised_device_republishes_status_at_a_bounded_rate() {
        let mut device = connected(&["--telemetry-interval", "3600"]);
        assert!(!device.clock_synced());
        // The heartbeat would be five hours away; the unsynchronised rate is
        // what makes the edge resend `edge.time`.
        let mut publications = 0;
        for _ in 0..600 {
            publications += status_count(&device.tick(1_000));
        }
        assert_eq!(
            publications, 10,
            "one status per 60 s while unsynchronised, and no more"
        );
    }

    #[test]
    fn a_synchronised_device_does_not_republish_every_minute() {
        let mut device = connected(&["--telemetry-interval", "3600"]);
        device.on_message(&time_topic(), &time_payload("plant-node-01", 1_000));
        let mut publications = 0;
        for _ in 0..600 {
            publications += status_count(&device.tick(1_000));
        }
        assert_eq!(publications, 0, "the heartbeat is five hours away");
    }

    // ------------------------------------------------------------- config

    #[test]
    fn a_valid_config_is_applied_echoed_and_changes_behaviour() {
        let mut device = connected(&[]);
        let published = device.on_message(&config_topic(), &config_payload("plant-node-01", 7, 60));
        assert_eq!(published.len(), 1, "a config change republishes status");
        assert_eq!(decode(&published[0]).data.applied_config_version, Some(7));
        assert_eq!(
            device.config().heartbeat_interval_ms(),
            300_000,
            "the new interval really drives the schedule"
        );
    }

    #[test]
    fn an_invalid_config_is_rejected_and_the_previous_one_is_reported() {
        let mut device = connected(&[]);
        device.on_message(&config_topic(), &config_payload("plant-node-01", 7, 60));
        let before = *device.config();

        // 9 seconds is below the protocol's 10 s minimum.
        let published = device.on_message(&config_topic(), &config_payload("plant-node-01", 8, 9));
        assert!(published.is_empty(), "a rejection publishes no new status");
        assert_eq!(*device.config(), before, "nothing may be half-applied");
        assert_eq!(device.config().applied_version, Some(7));
    }

    #[test]
    fn a_config_at_or_below_the_applied_version_is_ignored() {
        let mut device = connected(&[]);
        device.on_message(&config_topic(), &config_payload("plant-node-01", 7, 60));
        for version in [1, 7] {
            let published = device.on_message(
                &config_topic(),
                &config_payload("plant-node-01", version, 30),
            );
            assert!(published.is_empty());
            assert_eq!(device.config().telemetry_interval_seconds, 60);
        }
    }

    #[test]
    fn a_config_cannot_raise_a_reported_limit() {
        let mut device = connected(&[]);
        let smuggled = envelope_payload(
            "plant-node-01",
            "device.config",
            serde_json::json!({
                "config_version": 3,
                "telemetry_interval_seconds": 60,
                "pump": {"ml_per_second": 8.2, "enabled": true},
                "tank": {"min_percent": 15.0},
                "max_ml_per_run": 9999.0,
                "max_daily_ml": 100000.0,
                "max_run_seconds": 3600
            }),
        );
        let published = device.on_message(&config_topic(), &smuggled);
        let limits = decode(&published[0]).data.limits.unwrap();
        assert_eq!(limits.max_ml_per_run, FIRMWARE_MAX_ML_PER_RUN);
        assert_eq!(limits.max_daily_ml, FIRMWARE_MAX_DAILY_ML);
        assert_eq!(limits.max_run_seconds, FIRMWARE_MAX_RUN_SECONDS);
    }

    #[test]
    fn a_config_addressed_to_another_device_is_rejected_whole() {
        let mut device = connected(&[]);
        let published = device.on_message(&config_topic(), &config_payload("plant-node-02", 7, 60));
        assert!(published.is_empty());
        assert_eq!(
            device.config().applied_version,
            None,
            "a mismatched identity is untrustworthy in its entirety (protocol §10)"
        );
    }

    /// Deleting a retained message is a zero-length publish, and the broker
    /// delivers it to current subscribers. A device must treat it as "nothing
    /// to apply", not as a configuration.
    #[test]
    fn an_empty_payload_is_ignored_and_leaves_the_configuration_alone() {
        let mut device = connected(&[]);
        device.on_message(&config_topic(), &config_payload("plant-node-01", 7, 60));
        assert_eq!(device.config().telemetry_interval_seconds, 60);

        assert!(device.on_message(&config_topic(), b"").is_empty());
        assert_eq!(
            device.config().telemetry_interval_seconds,
            60,
            "a retained-message deletion must not disturb the applied config"
        );
        assert_eq!(device.config().applied_version, Some(7));
    }

    #[test]
    fn malformed_and_wrong_version_payloads_are_rejected_without_panicking() {
        let mut device = connected(&[]);
        for payload in [
            b"not json".to_vec(),
            b"{}".to_vec(),
            serde_json::to_vec(&serde_json::json!({
                "v": 2, "kind": "device.config", "message_id": MessageId::from_uuid(Uuid::nil()),
                "device_id": "plant-node-01", "data": {}
            }))
            .unwrap(),
        ] {
            assert!(device.on_message(&config_topic(), &payload).is_empty());
        }
        assert_eq!(device.config().applied_version, None);
    }

    // ---------------------------------------------------------- edge.time

    #[test]
    fn the_first_edge_time_synchronises_and_announces_it() {
        let mut device = connected(&[]);
        assert!(!device.clock_synced());
        let published = device.on_message(
            &time_topic(),
            &time_payload("plant-node-01", 1_756_121_400_000),
        );
        assert_eq!(published.len(), 1, "the transition is worth announcing");
        let status = decode(&published[0]);
        assert_eq!(status.clock_synced, Some(true));
        assert_eq!(status.device_time_ms, Some(UtcMillis(1_756_121_400_000)));
        assert_eq!(
            status.message_id.as_uuid().get_version_num(),
            7,
            "a synchronised device emits UUIDv7 (F-020-06)"
        );
        assert!(device.clock_synced());
    }

    /// SAFETY-002. A redelivered `edge.time` must not keep the clock alive.
    #[test]
    fn a_duplicate_edge_time_is_ignored_and_does_not_extend_the_window() {
        let mut device = connected(&[]);
        device.on_message(&time_topic(), &time_payload("plant-node-01", 1_000));
        // Redeliver the same value repeatedly across the whole validity window.
        for _ in 0..30 {
            device.tick(60_000);
            assert!(
                device
                    .on_message(&time_topic(), &time_payload("plant-node-01", 1_000))
                    .is_empty(),
                "a duplicate changes nothing"
            );
        }
        assert!(
            !device.clock_synced(),
            "a replayed message must not hold clock_synced true forever"
        );
    }

    #[test]
    fn an_older_edge_time_never_moves_the_clock_backwards() {
        let mut device = connected(&[]);
        device.on_message(&time_topic(), &time_payload("plant-node-01", 1_000));
        assert!(
            device
                .on_message(&time_topic(), &time_payload("plant-node-01", 999))
                .is_empty()
        );
        device.tick(1_000);
        let published = device.tick(TimeSync::max_age_ms() * 2);
        let status = statuses(&published).pop().expect("a status is due");
        // The wall time is derived from 1_000, never from 999.
        assert!(status.device_time_ms.is_none_or(|t| t.0 >= 1_000));
    }

    #[test]
    fn a_strictly_newer_edge_time_is_applied() {
        let mut device = connected(&[]);
        device.on_message(&time_topic(), &time_payload("plant-node-01", 1_000));
        assert!(
            device
                .on_message(&time_topic(), &time_payload("plant-node-01", 1_001))
                .is_empty(),
            "already synchronised, so no transition to announce"
        );
        assert!(device.clock_synced());
    }

    #[test]
    fn synchronisation_ages_out_and_the_device_says_so() {
        let mut device = connected(&["--telemetry-interval", "10"]);
        device.on_message(&time_topic(), &time_payload("plant-node-01", 1_000));
        assert!(device.clock_synced());

        let published = device.tick(TimeSync::max_age_ms());
        assert!(!device.clock_synced());
        let status = statuses(&published).pop().expect("the lapse is announced");
        assert_eq!(status.clock_synced, Some(false));
        assert_eq!(
            status.device_time_ms, None,
            "an unsynchronised device asserts no wall time"
        );
        assert_eq!(
            status.message_id.as_uuid().get_version_num(),
            4,
            "and emits a UUIDv4 rather than claiming a time-ordered id"
        );
    }

    #[test]
    fn clock_unsync_fault_is_visible_in_retained_status() {
        let mut device = connected(&[]);
        device.on_message(&time_topic(), &time_payload("plant-node-01", 1_000));
        assert!(device.clock_synced());

        device.enable_fault(Fault::ClockUnsync);
        let status = decode(
            &device
                .status_publication(DeviceStatusValue::Online, None)
                .expect("status serialises"),
        );

        assert_eq!(status.clock_synced, Some(false));
        assert_eq!(status.device_time_ms, None);
        assert_eq!(status.message_id.as_uuid().get_version_num(), 4);
    }

    #[test]
    fn disabling_a_parameterised_disconnect_ends_isolation_immediately() {
        let mut device = connected(&[]);
        device.enable_fault(Fault::Disconnect { seconds: 7_200 });
        assert!(device.is_isolated());

        device.disable_fault("disconnect:1");

        assert_eq!(device.isolation_remaining_ms(), 0);
        assert!(!device.faults().is_enabled("disconnect"));
    }

    #[test]
    fn a_fresh_synchronisation_after_a_lapse_restores_the_clock() {
        let mut device = connected(&[]);
        device.on_message(&time_topic(), &time_payload("plant-node-01", 1_000));
        device.tick(TimeSync::max_age_ms() + 1);
        assert!(!device.clock_synced());
        let published = device.on_message(&time_topic(), &time_payload("plant-node-01", 2_000));
        assert_eq!(published.len(), 1);
        assert!(device.clock_synced());
    }

    #[test]
    fn a_disconnect_and_reconnect_does_not_invent_a_clock() {
        let mut device = connected(&[]);
        device.on_message(&time_topic(), &time_payload("plant-node-01", 1_000));
        device.on_disconnected();
        device.tick(TimeSync::max_age_ms() + 1);
        let reconnected = device.on_connected().unwrap();
        assert_eq!(decode(&reconnected[0]).clock_synced, Some(false));
        // ...and a fresh sync after the reconnect restores it.
        device.on_message(&time_topic(), &time_payload("plant-node-01", 2_000));
        assert!(device.clock_synced());
    }

    #[test]
    fn an_edge_time_for_another_device_is_ignored() {
        let mut device = connected(&[]);
        assert!(
            device
                .on_message(&time_topic(), &time_payload("plant-node-02", 1_000))
                .is_empty()
        );
        assert!(!device.clock_synced());
    }

    // ---------------------------------------------------------- telemetry

    #[test]
    fn each_sampling_cycle_produces_exactly_one_valid_never_retained_batch() {
        let mut device = connected(&["--telemetry-interval", "10"]);
        let published = device.tick(10_000);
        let batches = telemetry(&published);
        assert_eq!(batches.len(), 1, "one batch per cycle, never split");
        assert!(batches[0].data.validate().is_ok());
        let p = published
            .iter()
            .find(|p| matches!(p.topic, Topic::Telemetry(_)))
            .unwrap();
        assert!(
            !p.retain,
            "a retained sample would be served to every new subscriber as current"
        );
        assert_eq!(p.qos, rhizo_mqtt_contract::Qos::One);
    }

    #[test]
    fn the_batch_contains_all_and_only_the_declared_kinds() {
        let mut device = connected(&["--telemetry-interval", "10", "--sensors", "soil,tank"]);
        let published = device.tick(10_000);
        let kinds: Vec<_> = telemetry(&published)[0]
            .data
            .samples
            .iter()
            .map(|s| s.kind.clone())
            .collect();
        assert_eq!(kinds, device.capabilities().sampled_kinds());
        assert!(
            !kinds.contains(&rhizo_mqtt_contract::payload::MeasurementKind::LeakState),
            "a disabled sensor contributes no sample"
        );
    }

    #[test]
    fn sequence_increases_monotonically_across_every_kind_of_publication() {
        let mut device = connected(&["--telemetry-interval", "10"]);
        let mut last = 0;
        for _ in 0..20 {
            for p in device.tick(10_000) {
                let value: serde_json::Value = serde_json::from_str(&p.payload).unwrap();
                let sequence = value["sequence"].as_u64().unwrap();
                assert!(sequence > last, "{sequence} did not advance past {last}");
                last = sequence;
            }
        }
    }

    // ------------------------------------------------- event acknowledgement

    fn ack_topic() -> Topic {
        Topic::EventsAck(DeviceId::parse("plant-node-01").unwrap())
    }

    fn ack_payload(boot_id: BootId, through_device_seq: u64) -> Vec<u8> {
        envelope_payload(
            "plant-node-01",
            "event.ack",
            serde_json::json!({
                "boot_id": boot_id,
                "through_device_seq": through_device_seq,
            }),
        )
    }

    /// Buffers `n` audit events and returns the sequences they were given.
    fn buffer_events(device: &mut Device, n: usize) -> Vec<u64> {
        (0..n)
            .map(|_| {
                device
                    .record_event(
                        EventTier::Audit,
                        EventKind::PolicyActivated,
                        EventDetail::PolicyActivated { policy_version: 1 },
                    )
                    .device_seq()
            })
            .collect()
    }

    fn replayed_ids(device: &Device) -> Vec<EventId> {
        device.buffered_event_ids()
    }

    /// A replay is a retransmission, not a handover: until the edge says it has
    /// the events, the device keeps them and sends them again.
    #[test]
    fn a_replay_discards_nothing_until_an_acknowledgement_arrives() {
        let mut device = connected(&[]);
        buffer_events(&mut device, 5);

        for _ in 0..3 {
            device.on_disconnected();
            let published = device.on_connected().unwrap();
            let replayed: usize = published
                .iter()
                .filter(|p| matches!(p.topic, Topic::Events(_)))
                .map(|p| {
                    serde_json::from_str::<serde_json::Value>(&p.payload).unwrap()["data"]["events"]
                        .as_array()
                        .map_or(0, Vec::len)
                })
                .sum();
            assert_eq!(replayed, 5, "every reconnection replays the whole buffer");
            assert_eq!(device.buffered_events(), 5);
        }
        assert_eq!(device.acknowledged_through(), None);
    }

    #[test]
    fn an_acknowledgement_discards_the_covered_prefix() {
        let mut device = connected(&[]);
        let seqs = buffer_events(&mut device, 6);
        let boot = device.boot_id();

        let published = device.on_message(&ack_topic(), &ack_payload(boot, seqs[2]));
        assert!(
            published.is_empty(),
            "an acknowledgement is not itself acknowledged"
        );
        assert_eq!(
            device.last_ack_outcome(),
            Some(AckOutcome::Applied {
                through_seq: seqs[2],
                removed: 3,
            })
        );
        assert_eq!(device.buffered_events(), 3);
        assert_eq!(device.acknowledged_through(), Some(seqs[2]));
    }

    /// The edge published an acknowledgement, the device never saw it, and the
    /// edge moved on. The device replays what it still holds; the edge
    /// deduplicates on `event_id`. Losing an acknowledgement costs bandwidth,
    /// never history.
    #[test]
    fn a_lost_acknowledgement_costs_a_replay_and_nothing_else() {
        let mut device = connected(&[]);
        let seqs = buffer_events(&mut device, 4);
        let ids = replayed_ids(&device);

        // The acknowledgement is dropped in flight — simply never delivered.
        device.on_disconnected();
        let _ = device.on_connected().unwrap();

        assert_eq!(device.buffered_events(), 4);
        assert_eq!(
            replayed_ids(&device),
            ids,
            "the same events with the same ids, so the edge can deduplicate"
        );
        assert_eq!(device.acknowledged_through(), None);

        // And the retry lands.
        let boot = device.boot_id();
        device.on_message(&ack_topic(), &ack_payload(boot, seqs[3]));
        assert_eq!(device.buffered_events(), 0);
    }

    #[test]
    fn a_duplicate_acknowledgement_changes_nothing() {
        let mut device = connected(&[]);
        let seqs = buffer_events(&mut device, 5);
        let boot = device.boot_id();

        device.on_message(&ack_topic(), &ack_payload(boot, seqs[1]));
        let after_first = device.buffered_events();
        for _ in 0..3 {
            device.on_message(&ack_topic(), &ack_payload(boot, seqs[1]));
            assert_eq!(
                device.last_ack_outcome(),
                Some(AckOutcome::NotNewer {
                    through_seq: seqs[1]
                })
            );
        }
        assert_eq!(device.buffered_events(), after_first);
    }

    #[test]
    fn an_older_acknowledgement_never_un_acknowledges_history() {
        let mut device = connected(&[]);
        let seqs = buffer_events(&mut device, 6);
        let boot = device.boot_id();

        device.on_message(&ack_topic(), &ack_payload(boot, seqs[4]));
        assert_eq!(device.acknowledged_through(), Some(seqs[4]));
        device.on_message(&ack_topic(), &ack_payload(boot, seqs[1]));
        assert_eq!(
            device.acknowledged_through(),
            Some(seqs[4]),
            "a delayed, lower acknowledgement is a no-op, not a rewind"
        );
        assert_eq!(device.buffered_events(), 1);
    }

    /// Fail-closed: an acknowledgement the device cannot match to anything it
    /// issued deletes nothing at all.
    #[test]
    fn an_acknowledgement_beyond_known_state_deletes_nothing() {
        let mut device = connected(&[]);
        let seqs = buffer_events(&mut device, 3);
        let boot = device.boot_id();
        let highest = *seqs.last().unwrap();

        for beyond in [highest + 1, u64::MAX] {
            device.on_message(&ack_topic(), &ack_payload(boot, beyond));
            assert_eq!(
                device.last_ack_outcome(),
                Some(AckOutcome::BeyondKnown {
                    through_seq: beyond,
                    highest,
                })
            );
            assert_eq!(device.buffered_events(), 3, "nothing was deleted");
            assert_eq!(device.acknowledged_through(), None);
        }
    }

    /// An acknowledgement names a boot. One from a previous boot says nothing
    /// about the history *this* boot holds — sequences continue across
    /// restarts, so honouring a stale acknowledgement would delete events
    /// buffered since it was sent.
    #[test]
    fn an_acknowledgement_for_another_boot_is_ignored() {
        let mut device = connected(&[]);
        buffer_events(&mut device, 4);
        let stranger = BootId::from_uuid(Uuid::from_u128(0x0123_4567_89ab_cdef));

        device.on_message(&ack_topic(), &ack_payload(stranger, 2));
        assert_eq!(
            device.last_ack_outcome(),
            None,
            "a mismatched boot is refused before the buffer is touched"
        );
        assert_eq!(device.buffered_events(), 4);
        assert_eq!(device.acknowledged_through(), None);
    }

    /// The acknowledgement is persisted with the buffer it trimmed, so a
    /// restart does not resurrect history the edge already has.
    #[test]
    fn an_acknowledgement_survives_a_restart() {
        let state = crate::testutil::scratch_state_file();
        let path = state.display().to_string();
        let args = ["--state-file", path.as_str()];

        let seqs = {
            let mut device = connected(&args);
            let seqs = buffer_events(&mut device, 6);
            let boot = device.boot_id();
            device.on_message(&ack_topic(), &ack_payload(boot, seqs[3]));
            assert_eq!(device.buffered_events(), 2);
            seqs
        };

        let restarted = connected(&args);
        assert_eq!(
            restarted.buffered_events(),
            2,
            "the trimmed buffer is what was persisted"
        );
        assert_eq!(restarted.acknowledged_through(), Some(seqs[3]));
    }

    /// A gap marker can only be acknowledged once the edge has actually been
    /// told about the gap — which means after the marker has been sealed and
    /// replayed. Nothing about a run of losses is discarded on the strength of
    /// an acknowledgement that predates the device saying anything about it.
    #[test]
    fn a_gap_is_only_acknowledgeable_once_the_edge_has_been_told_about_it() {
        let mut device = connected(&[]);
        buffer_events(&mut device, crate::buffer::AUDIT_CAPACITY + 3);
        let boot = device.boot_id();

        // Acknowledge everything issued so far. The marker has never been sent,
        // so it survives and is still replayed.
        let highest = device.highest_allocated_seq().unwrap();
        device.on_message(&ack_topic(), &ack_payload(boot, highest));

        device.on_disconnected();
        let published = device.on_connected().unwrap();
        let markers: Vec<serde_json::Value> = published
            .iter()
            .filter(|p| matches!(p.topic, Topic::Events(_)))
            .flat_map(|p| {
                serde_json::from_str::<serde_json::Value>(&p.payload).unwrap()["data"]["events"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .filter(|e| e["kind"] == "history.gap")
            .collect();
        assert_eq!(
            markers.len(),
            1,
            "the loss is reported even though the sweep covered its sequence"
        );

        // Now that it has been sent, it is acknowledgeable like anything else.
        let marker_seq = markers[0]["device_seq"].as_u64().unwrap();
        device.on_message(&ack_topic(), &ack_payload(boot, marker_seq));
        assert_eq!(device.buffered_events(), 0);
    }

    #[test]
    fn a_boot_id_is_fresh_and_shared_by_every_publication_of_that_boot() {
        let mut first = connected(&["--telemetry-interval", "10"]);
        let published = first.tick(10_000);
        let boot: std::collections::HashSet<_> = published
            .iter()
            .map(|p| {
                serde_json::from_str::<serde_json::Value>(&p.payload).unwrap()["boot_id"].clone()
            })
            .collect();
        assert_eq!(boot.len(), 1, "one boot, one boot_id");

        let mut second = connected(&["--telemetry-interval", "10"]);
        let other = serde_json::from_str::<serde_json::Value>(&second.tick(10_000)[0].payload)
            .unwrap()["boot_id"]
            .clone();
        assert_ne!(
            boot.into_iter().next().unwrap(),
            other,
            "a restart must produce a fresh boot_id"
        );
    }

    #[test]
    fn a_disconnect_buffers_at_most_sixteen_cycles_and_replays_them_in_order() {
        let mut device = connected(&["--telemetry-interval", "10"]);
        device.on_disconnected();
        // Ten minutes at a ten-second interval is sixty cycles.
        for _ in 0..60 {
            assert!(
                telemetry(&device.tick(10_000)).is_empty(),
                "nothing is published while disconnected"
            );
        }
        assert_eq!(
            device.buffered_cycles(),
            crate::telemetry::TELEMETRY_RING,
            "a device is not a ledger for samples"
        );

        let published = device.on_connected().unwrap();
        let batches = telemetry(&published);
        assert_eq!(batches.len(), crate::telemetry::TELEMETRY_RING);
        let sequences: Vec<_> = batches.iter().map(|b| b.sequence.unwrap()).collect();
        let mut sorted = sequences.clone();
        sorted.sort_unstable();
        assert_eq!(sequences, sorted, "buffered cycles replay oldest first");
        assert_eq!(device.buffered_cycles(), 0);
    }

    #[test]
    fn a_leak_change_publishes_immediately_rather_than_waiting_for_the_schedule() {
        let mut device = connected(&["--telemetry-interval", "3600"]);
        device.tick(1_000);
        assert!(
            telemetry(&device.tick(1_000)).is_empty(),
            "the next scheduled cycle is an hour away"
        );

        device.environment_mut().tank.set_leak(LeakState::Detected);
        let published = device.tick(1_000);
        let batches = telemetry(&published);
        assert_eq!(
            batches.len(),
            1,
            "an hour-late leak notification is useless"
        );
        let leak = batches[0]
            .data
            .samples
            .iter()
            .find(|s| s.kind == rhizo_mqtt_contract::payload::MeasurementKind::LeakState)
            .unwrap();
        assert_eq!(
            leak.value,
            Some(rhizo_mqtt_contract::payload::MeasurementValue::Boolean(
                true
            ))
        );

        assert!(
            telemetry(&device.tick(1_000)).is_empty(),
            "and the schedule resumes rather than free-running"
        );
    }

    #[test]
    fn a_device_with_no_sensors_publishes_no_batch_at_all() {
        let mut device = connected(&["--telemetry-interval", "10", "--sensors", ""]);
        for _ in 0..10 {
            assert!(
                telemetry(&device.tick(10_000)).is_empty(),
                "an empty batch MUST NOT be published"
            );
        }
        assert_eq!(device.buffered_cycles(), 0);
    }

    // ------------------------------------------------------------ actuator

    fn actuators(published: &[Publication]) -> Vec<Envelope<ActuatorState>> {
        published
            .iter()
            .filter(|p| matches!(p.topic, Topic::Actuator(_)))
            .map(|p| {
                let decoded = Envelope::<ActuatorState>::from_json(p.payload.as_bytes()).unwrap();
                decoded.check_topic(&p.topic).unwrap();
                decoded
            })
            .collect()
    }

    #[test]
    fn actuator_state_is_published_on_change_and_not_periodically() {
        let mut device = connected(&["--telemetry-interval", "10"]);
        let first = actuators(&device.tick(1_000));
        assert_eq!(first.len(), 1, "the initial state is a change from nothing");
        assert!(!first[0].data.active);
        assert!(!first[0].data.faulted);
        assert_eq!(first[0].data.actuator_id.as_str(), "pump-0");

        for _ in 0..50 {
            assert!(
                actuators(&device.tick(1_000)).is_empty(),
                "actuator state is state, not a measurement"
            );
        }

        device.set_actuator_faulted(true);
        let changed = actuators(&device.tick(1_000));
        assert_eq!(changed.len(), 1);
        assert!(changed[0].data.faulted);
    }

    #[test]
    fn actuator_state_is_never_retained() {
        let mut device = connected(&[]);
        let published = device.tick(1_000);
        let p = published
            .iter()
            .find(|p| matches!(p.topic, Topic::Actuator(_)))
            .unwrap();
        assert!(!p.retain);
    }

    #[test]
    fn a_monitoring_only_device_publishes_no_actuator_state_ever() {
        let mut device = connected(&["--telemetry-interval", "10", "--actuators", ""]);
        for _ in 0..20 {
            assert!(
                actuators(&device.tick(10_000)).is_empty(),
                "a plant with no actuator has no actuator state to report"
            );
        }
        assert!(device.capabilities().actuators().is_empty());
    }

    #[test]
    fn status_declares_exactly_the_capabilities_that_are_sampled() {
        let mut device = connected(&["--telemetry-interval", "10", "--sensors", "soil,leak"]);
        let status = statuses(&device.on_connected().unwrap()).pop().unwrap();
        let declared: Vec<_> = status
            .data
            .capabilities
            .sensors
            .iter()
            .flat_map(|s| s.kinds.clone())
            .collect();
        let sampled: Vec<_> = telemetry(&device.tick(10_000))[0]
            .data
            .samples
            .iter()
            .map(|s| s.kind.clone())
            .collect();
        assert_eq!(declared, sampled);
    }
}
