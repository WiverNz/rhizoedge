//! Simulator/firmware conformance (M9-014, ADR-008 mechanism 5).
//!
//! The same scenario driven against the reference simulator and against the
//! firmware application with fake adapters must produce the same answers. This
//! is what catches behavioural divergence the type system cannot — and it is
//! what makes M6's simulator-based safety tests transfer to hardware, because
//! otherwise they only ever prove things about the simulator.
//!
//! # What is compared, and what is not
//!
//! **Decisions, not bytes and not timing.** `message_id`, `command_id`, and
//! every timestamp differ on every run, and the two physical models differ
//! deliberately — the simulator has a soil model and an evaporation rate, the
//! firmware has fake adapters. A byte comparison would fail always and prove
//! nothing.
//!
//! What is compared is the `command.result` a given set of *gate inputs*
//! produces: status, reason, clamped, delivered volume, and the running daily
//! total. The sensor readings are taken **from the simulator** and fed to the
//! firmware, so the comparison isolates the decision from the model that
//! produced its inputs.
//!
//! # The reject cases matter most
//!
//! An oversized command, an expired command, a leak, an empty tank, and a
//! duplicate must produce the same reason from both. That is precisely the set
//! M6's safety tests assert against the simulator.
//!
//! # If the two disagree
//!
//! The question is which is canonical, not how to make the test accept both.
//! The gate is `rhizo_mqtt_contract::validate_water_command` and there is one
//! of it; a disagreement means one of the two callers is feeding it different
//! inputs, and the fix is in that caller.

// A panic in a test is a failed assertion, not an unhandled failure: the
// workspace denies `unwrap`/`expect` in library code, and an integration test
// is a separate crate that does not inherit the `cfg(test)` allowance in
// `lib.rs` (workspace lint policy, root Cargo.toml).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use device_simulator::{Cli, Device, Fault};
use rhizo_mqtt_contract::payload::{CommandResult, CommandStatus, RejectReason, WaterCommand};
use rhizo_mqtt_contract::safety::LeakState;
use rhizo_mqtt_contract::{CommandId, DeviceId, Envelope, MessageId, Topic, UtcMillis};
use rhizo_node_app::command::{GateInputs, handle_water};
use rhizo_node_app::fakes::{FakeNvs, FakePump, call_log};
use rhizo_node_app::persist::PersistedState;

use clap::Parser;
use uuid::Uuid;

/// The pump calibration both implementations run with.
const ML_PER_SECOND: f32 = 8.0;
/// The reservoir minimum both implementations run with (the shared default).
const TANK_MIN_PERCENT: f32 = 15.0;
/// The wall time the edge hands out.
const EDGE_TIME_MS: i64 = 1_756_121_400_000;

/// The comparable content of one `command.result`.
///
/// Everything that is a *decision*, and nothing that is an identifier or an
/// instant.
#[derive(Debug, PartialEq)]
struct Decision {
    status: CommandStatus,
    reason: Option<RejectReason>,
    clamped: bool,
    delivered_ml: Option<f32>,
    delivered_today_ml: f32,
    origin: rhizo_mqtt_contract::payload::CommandOrigin,
}

impl From<&CommandResult> for Decision {
    fn from(result: &CommandResult) -> Self {
        Self {
            status: result.status,
            reason: result.reason,
            clamped: result.clamped,
            delivered_ml: result.delivered_ml,
            delivered_today_ml: result.delivered_today_ml,
            origin: result.origin,
        }
    }
}

fn device_id() -> DeviceId {
    DeviceId::parse("plant-node-01").expect("valid device id")
}

fn scratch_state_file(name: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push("rhizo-conformance");
    let _ = std::fs::create_dir_all(&path);
    path.push(format!("{}-{name}.state.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

/// A simulator configured to match the firmware's calibration.
fn simulator(name: &str) -> Device {
    let state_file = scratch_state_file(name);
    let cli = Cli::try_parse_from([
        "device-simulator",
        "--device-id",
        "plant-node-01",
        "--ml-per-second",
        "8.0",
        "--sensors",
        "soil,tank,leak,weight",
        "--no-noise",
        "--seed",
        "7",
        "--no-control-api",
        "--state-file",
        &state_file.display().to_string(),
    ])
    .expect("conformance arguments parse");
    cli.validate().expect("conformance arguments validate");
    let mut device = Device::new(&cli);
    // The simulator queues results while it believes it is offline and
    // publishes them on connect, so a conformance run has to connect first —
    // otherwise it is comparing the firmware against a device that is buffering
    // rather than answering.
    device
        .on_connected()
        .expect("the simulator publishes its retained status on connect");
    device
}

/// A distinct `message_id` per envelope.
///
/// A counter rather than a random UUID: the comparison ignores the value, and a
/// deterministic one makes a failing run reproducible.
fn next_message_id() -> MessageId {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    MessageId::from_uuid(Uuid::from_u128(u128::from(
        NEXT.fetch_add(1, Ordering::Relaxed),
    )))
}

fn envelope<T: serde::Serialize>(kind: rhizo_mqtt_contract::MessageKind, data: T) -> String {
    Envelope {
        v: 1,
        kind,
        message_id: next_message_id(),
        device_id: device_id(),
        boot_id: None,
        sequence: None,
        device_time_ms: None,
        clock_synced: None,
        data,
    }
    .to_json()
    .expect("envelope encodes")
}

/// Synchronises the simulator's clock, as the edge does on receiving a status.
fn sync_clock(device: &mut Device) {
    let payload = envelope(
        rhizo_mqtt_contract::MessageKind::EdgeTime,
        rhizo_mqtt_contract::payload::EdgeTime {
            edge_time_ms: UtcMillis(EDGE_TIME_MS),
        },
    );
    device.on_message(&Topic::Time(device_id()), payload.as_bytes());
    assert!(device.clock_synced(), "the simulator accepted edge.time");
}

/// Delivers one `command.water` to the simulator and returns its result.
///
/// The pump runs on virtual time, so the tick after the command is what turns
/// an accepted dose into a delivered one. A rejection publishes its result
/// immediately and the tick simply finds nothing to do.
/// The device time carried by the envelope, which an **unsynchronised** device
/// omits entirely — so the caller gets an `Option` and cannot mistake "the
/// device does not know what time it is" for a timestamp.
fn simulator_result(
    device: &mut Device,
    command: &WaterCommand,
) -> (CommandResult, Option<UtcMillis>) {
    let payload = envelope(rhizo_mqtt_contract::MessageKind::CommandWater, command);
    let mut publications = device.on_message(&Topic::CommandWater(device_id()), payload.as_bytes());
    publications.extend(device.tick(30_000));
    publications
        .iter()
        .filter(|p| matches!(p.topic, Topic::CommandResult(_)))
        .map(|p| {
            let envelope = Envelope::<CommandResult>::from_json(p.payload.as_bytes())
                .expect("the simulator publishes a decodable result");
            (envelope.data, envelope.device_time_ms)
        })
        .find(|(result, _)| result.command_id == command.command_id)
        .unwrap_or_else(|| panic!("no result for {}", command.command_id))
}

/// The gate inputs the simulator itself is holding.
///
/// Read from the simulator rather than assumed, so the comparison is about the
/// *decision* and not about whether two soil models agree.
fn inputs_from(device: &Device, now_ms: UtcMillis) -> GateInputs {
    GateInputs {
        clock_synced: device.clock_synced(),
        now_ms,
        leak: device.leak_reading().unwrap_or(LeakState::Unknown),
        tank_percent: device.tank_reading(),
        tank_min_percent: TANK_MIN_PERCENT,
        pump_enabled: true,
        pump_faulted: !device.pump_health().unwrap_or(false),
        pump_ml_per_second: ML_PER_SECOND,
    }
}

struct Firmware {
    state: PersistedState,
    nvs: FakeNvs,
    pump: FakePump,
}

impl Firmware {
    fn new() -> Self {
        Self {
            state: PersistedState::default(),
            nvs: FakeNvs::new(),
            pump: FakePump::new(call_log()),
        }
    }

    fn result(&mut self, command: &WaterCommand, inputs: &GateInputs) -> CommandResult {
        handle_water(
            &mut self.state,
            &mut self.nvs,
            &mut self.pump,
            command,
            inputs,
        )
        .result
    }
}

fn command(n: u128, requested_ml: f32) -> WaterCommand {
    WaterCommand {
        command_id: CommandId::from_uuid(Uuid::from_u128(n)),
        requested_ml,
        issued_at_ms: UtcMillis(EDGE_TIME_MS),
        expires_at_ms: UtcMillis(EDGE_TIME_MS + 120_000),
    }
}

/// Runs one command against both implementations and compares the decision.
fn compare(name: &str, faults: &[Fault], command: &WaterCommand, synced: bool) -> Decision {
    let mut device = simulator(name);
    for fault in faults {
        device.enable_fault(*fault);
    }
    if synced {
        sync_clock(&mut device);
    }
    let inputs = inputs_from(&device, UtcMillis(EDGE_TIME_MS));
    let (simulated, _) = simulator_result(&mut device, command);

    let mut firmware = Firmware::new();
    let actual = firmware.result(command, &inputs);

    let expected = Decision::from(&simulated);
    let observed = Decision::from(&actual);
    assert_eq!(
        expected, observed,
        "{name}: simulator and firmware disagree.\n  simulator: {simulated:?}\n  firmware:  {actual:?}\n  inputs:    {inputs:?}"
    );
    observed
}

#[test]
fn conformance_accepted_dose() {
    let decision = compare("accept", &[], &command(1, 40.0), true);
    assert_eq!(decision.status, CommandStatus::Completed);
    assert_eq!(decision.delivered_ml, Some(40.0));
}

/// SAFETY-007 through both implementations, from the same shared clamp.
#[test]
fn conformance_oversized_command_is_clamped_identically() {
    let decision = compare("oversized", &[], &command(2, 5_000.0), true);
    assert!(decision.clamped);
    assert_eq!(
        decision.delivered_ml,
        Some(rhizo_mqtt_contract::safety::FIRMWARE_MAX_ML_PER_RUN)
    );
}

/// SAFETY-002. The refusal an unsynchronised device gives is the one the edge
/// keys its retry off, so the two must agree on it exactly.
#[test]
fn conformance_clock_unsynced_refusal() {
    let decision = compare("unsynced", &[], &command(3, 40.0), false);
    assert_eq!(decision.status, CommandStatus::Rejected);
    assert_eq!(decision.reason, Some(RejectReason::ClockUnsynced));
}

#[test]
fn conformance_expired_command_refusal() {
    let mut expired = command(4, 40.0);
    expired.issued_at_ms = UtcMillis(EDGE_TIME_MS - 600_000);
    expired.expires_at_ms = UtcMillis(EDGE_TIME_MS - 480_000);
    let decision = compare("expired", &[], &expired, true);
    assert_eq!(decision.reason, Some(RejectReason::Expired));
}

#[test]
fn conformance_malformed_command_refusal() {
    let decision = compare("malformed", &[], &command(5, 0.0), true);
    assert_eq!(decision.reason, Some(RejectReason::MalformedCommand));
}

/// SAFETY-003.
#[test]
fn conformance_leak_refusal() {
    let decision = compare("leak", &[Fault::Leak], &command(6, 40.0), true);
    assert_eq!(decision.reason, Some(RejectReason::LeakDetected));
}

/// SAFETY-004.
#[test]
fn conformance_tank_low_refusal() {
    let decision = compare("tank", &[Fault::TankEmpty], &command(7, 40.0), true);
    assert_eq!(decision.reason, Some(RejectReason::TankLow));
}

/// SAFETY-001. A duplicate republishes the stored result and actuates nothing,
/// in both implementations, and the republished result is byte-for-byte the
/// decision the first one produced.
#[test]
fn conformance_duplicate_command_republishes_the_stored_result() {
    let mut device = simulator("duplicate");
    sync_clock(&mut device);
    let inputs = inputs_from(&device, UtcMillis(EDGE_TIME_MS));
    let command = command(8, 40.0);

    let (sim_first, _) = simulator_result(&mut device, &command);
    let (sim_second, _) = simulator_result(&mut device, &command);

    let mut firmware = Firmware::new();
    let fw_first = firmware.result(&command, &inputs);
    let ran_after_first = firmware.pump.total_run_ms;
    let fw_second = firmware.result(&command, &inputs);

    assert_eq!(Decision::from(&sim_first), Decision::from(&fw_first));
    assert_eq!(Decision::from(&sim_second), Decision::from(&fw_second));
    assert_eq!(
        Decision::from(&sim_first),
        Decision::from(&sim_second),
        "the simulator republishes the stored result unchanged"
    );
    assert_eq!(
        Decision::from(&fw_first),
        Decision::from(&fw_second),
        "the firmware republishes the stored result unchanged"
    );
    assert_eq!(
        firmware.pump.total_run_ms, ran_after_first,
        "a duplicate must not actuate"
    );
}

/// SAFETY-007's daily cap, reached the same way in both.
#[test]
fn conformance_over_daily_max_refusal() {
    let mut device = simulator("daily");
    sync_clock(&mut device);
    let mut firmware = Firmware::new();

    // Seven 80 ml doses is 560 ml against a 500 ml cap, so the seventh is the
    // one that must be refused — by both, with the same reason, after the same
    // six acceptances.
    //
    // The simulator runs on virtual time and each dose advances it, so the
    // wall clock is carried forward from the *simulator's own* `device_time_ms`
    // rather than held fixed. Holding it fixed made the sixth command expire on
    // the simulator's clock and not on the firmware's, which reads as a
    // divergence and is really the harness comparing two different instants.
    let mut refused_at = None;
    let mut now = UtcMillis(EDGE_TIME_MS);
    for n in 0..7u128 {
        let mut command = command(100 + n, 80.0);
        command.issued_at_ms = now;
        command.expires_at_ms = UtcMillis(now.0 + 120_000);
        let inputs = inputs_from(&device, now);
        let (simulated, at) = simulator_result(&mut device, &command);
        now = at.unwrap_or(now);
        let actual = firmware.result(&command, &inputs);
        assert_eq!(
            Decision::from(&simulated),
            Decision::from(&actual),
            "dose {n} diverged"
        );
        if actual.reason == Some(RejectReason::OverDailyMax) && refused_at.is_none() {
            refused_at = Some(n);
        }
    }
    assert_eq!(
        refused_at,
        Some(6),
        "the cap must be reached on the seventh dose in both"
    );
}

/// The device subscribes to exactly the eight exact topics, and both take the
/// set from the same shared constructor rather than listing it twice.
#[test]
fn conformance_subscription_set_is_identical() {
    let device = simulator("subs");
    assert_eq!(
        device.subscriptions().to_vec(),
        Topic::device_subscriptions(&device_id()).to_vec()
    );
    assert!(
        !device
            .subscriptions()
            .iter()
            .any(|t| t.ends_with("/commands/result")),
        "a device never subscribes to its own output"
    );
}

/// The refusal vocabulary an isolated device buffers must be identical, because
/// the edge stores these strings and an operator reads them.
///
/// Two independent `match` statements over the same shared enum, compared
/// exhaustively. A reason added to `rhizo-policy` fails to compile in both
/// until it is named, and this test fails if the two names disagree.
#[test]
fn conformance_offline_refusal_names_are_identical() {
    use rhizo_policy::RefuseReason;
    let all = [
        RefuseReason::NoValidPolicy,
        RefuseReason::PolicyDisabled,
        RefuseReason::PolicyInvalid,
        RefuseReason::NoActuator,
        RefuseReason::ControlMissing,
        RefuseReason::ControlStale,
        RefuseReason::ControlQuality,
        RefuseReason::ControlKindUnknown,
        RefuseReason::RequiredMissing,
        RefuseReason::RequiredStale,
        RefuseReason::RequiredQuality,
        RefuseReason::LeakDetected,
        RefuseReason::LeakUnknown,
        RefuseReason::TankUnknown,
        RefuseReason::TankLow,
        RefuseReason::PumpUnknown,
        RefuseReason::PumpUnhealthy,
        RefuseReason::CooldownActive,
        RefuseReason::BudgetExhausted,
        RefuseReason::MaxDosesReached,
    ];
    for reason in all {
        assert_eq!(
            device_simulator::offline::refuse_reason_name(reason),
            rhizo_node_app::offline::refuse_reason_name(reason),
            "{reason:?}"
        );
    }
}

/// The negative check M9-014 requires: a deliberate divergence must fail.
///
/// Injected by feeding the firmware inputs the simulator does not have — a
/// clear leak sensor where the simulator sees water. If the comparison were
/// vacuous this would still pass.
#[test]
fn conformance_detects_an_injected_divergence() {
    let mut device = simulator("divergence");
    device.enable_fault(Fault::Leak);
    sync_clock(&mut device);
    let mut diverged = inputs_from(&device, UtcMillis(EDGE_TIME_MS));
    assert_eq!(
        diverged.leak,
        LeakState::Detected,
        "the simulator sees water"
    );
    diverged.leak = LeakState::Clear;

    let command = command(200, 40.0);
    let (simulated, _) = simulator_result(&mut device, &command);
    let mut firmware = Firmware::new();
    let actual = firmware.result(&command, &diverged);

    assert_ne!(
        Decision::from(&simulated),
        Decision::from(&actual),
        "an injected divergence must be detected, not absorbed"
    );
}
