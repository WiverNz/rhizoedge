//! Command handling — the one path to actuation.
//!
//! **The requirement the simulator's usefulness depends on** (PRD 020
//! F-020-20): every command capable of moving the pump is decided by the shared
//! gate in `rhizo-mqtt-contract`, and there is exactly one call to it in this
//! crate. There is no `--allow-any-dose`, no debug bypass, and no test-only
//! relaxation. Removing that call would be a visible change to a
//! safety-critical file, and `safety_007_simulator_refuses_like_hardware`
//! asserts the refusal directly.
//!
//! A simulator more permissive than firmware makes every M6 safety test
//! meaningless: the suite would be validating a system that does not exist, and
//! the divergence would surface with real water and a real floor.
//!
//! # Order of operations
//!
//! ```text
//! decode → persistent-state fault? → shared gate → persist → actuate → result
//! ```
//!
//! The persistent-state check comes **before** the gate because it is a
//! different question: the gate decides whether this dose is safe, while the
//! fault says the device cannot trust what it knows about previous doses. The
//! persist step comes **before** actuation (protocol §5.8 step 13), so a
//! restart mid-dose is detectable on the next boot.
//!
//! # Calibration goes through the same gate
//!
//! `command.calibrate` runs the pump, so it is a dose. It is converted to the
//! volume it would deliver and put through the same function. See §5.9 in
//! [mqtt-v1.md](../../../../docs/protocol/mqtt-v1.md) for why the alternative —
//! a subset of the checks — is not implementable without the second rule set
//! ADR-008 forbids.

/// How long a device waits before republishing an unacknowledged result.
///
/// Protocol §5.10 used to bound the retry at 60 s because it was waiting for a
/// PUBACK. Waiting for the **edge** has no such bound: a result is retried for
/// as long as it is unacknowledged, and this is only how often.
pub const COMMAND_RESULT_RETRY_MS: u64 = 15_000;

/// How many unacknowledged results a device holds before evicting the oldest.
pub const PENDING_RESULT_LIMIT: usize = 32;

use rhizo_mqtt_contract::payload::{
    CalibrateCommand, CommandOrigin, CommandResult, CommandResultAck, CommandStatus, RejectReason,
    TareCommand, WaterCommand,
};
use rhizo_mqtt_contract::safety::{
    CommandVerdict, DeviceGuardState, LeakState, PreviousCommand, validate_water_command,
};
use rhizo_mqtt_contract::{CommandId, Topic, UtcMillis};

use crate::device::Device;
use crate::envelope::Publication;
use crate::state::{CommandRecord, InFlightDose};

/// The gate's verdict, detached from the borrow of the stored results.
///
/// [`validate_water_command`] hands back a reference into the dedup ring, and
/// acting on the verdict needs `&mut self`. Copying the answer out is what lets
/// the borrow end before anything is changed.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// The gate authorised a bounded dose.
    Accept {
        /// Volume authorised, after any clamp.
        effective_ml: f32,
        /// Duration authorised, after any clamp.
        run_ms: u32,
        /// Whether a hard limit changed the request.
        clamped: bool,
    },
    /// The gate refused, with the exact reason.
    Reject(RejectReason),
    /// This command has already been executed; republish the stored result.
    AlreadyExecuted(CommandResult),
}

impl Device {
    /// Handles `command.water`.
    pub(crate) fn on_water_command(&mut self, payload: &[u8]) -> Vec<Publication> {
        let Some(envelope) = self.decode_command::<WaterCommand>(payload, "command.water") else {
            return Vec::new();
        };
        let command = envelope.data;
        // Wire-level structure first: a command that is malformed on the wire
        // never reaches the gate, and is refused with the same vocabulary.
        if command.validate().is_err() {
            return self.reject(
                command.command_id,
                command.requested_ml,
                RejectReason::MalformedCommand,
                None,
            );
        }
        self.dose(command, CommandOrigin::EdgeCommand, None)
    }

    /// Handles `command.calibrate`.
    ///
    /// Converted to the volume it would deliver and put through the same gate.
    /// Its delivered volume counts toward the daily total exactly like any other
    /// dose (protocol §5.9).
    pub(crate) fn on_calibrate_command(&mut self, payload: &[u8]) -> Vec<Publication> {
        let Some(envelope) = self.decode_command::<CalibrateCommand>(payload, "command.calibrate")
        else {
            return Vec::new();
        };
        let command = envelope.data;
        let ml_per_second = self.config().pump.ml_per_second;
        if command.validate().is_err() || !ml_per_second.is_finite() || ml_per_second <= 0.0 {
            // An unusable calibration cannot be converted to a volume. The gate
            // would refuse it as `pump_unavailable` for the same reason, and
            // saying so here keeps the two answers identical.
            let reason = if command.validate().is_err() {
                RejectReason::MalformedCommand
            } else {
                RejectReason::PumpUnavailable
            };
            return self.reject(command.command_id, 0.0, reason, None);
        }
        let requested_ml = command.run_seconds * ml_per_second;
        self.dose(
            WaterCommand {
                command_id: command.command_id,
                requested_ml,
                issued_at_ms: command.issued_at_ms,
                expires_at_ms: command.expires_at_ms,
            },
            CommandOrigin::EdgeCommand,
            Some(String::from("calibrate")),
        )
    }

    /// Handles `command.tare`.
    ///
    /// **Not an actuation path.** Taring zeroes the scale and moves no water, so
    /// it does not go through the water gate — routing it there would mean
    /// inventing a volume for a command that has none. It is still deduplicated
    /// and still refused on an unsynchronised or expired basis, because a
    /// replayed tare would silently re-zero a scale mid-experiment.
    pub(crate) fn on_tare_command(&mut self, payload: &[u8]) -> Vec<Publication> {
        let Some(envelope) = self.decode_command::<TareCommand>(payload, "command.tare") else {
            return Vec::new();
        };
        let command = envelope.data;
        if let Some(previous) = self.stored_result(command.command_id) {
            return self.queue_result(previous);
        }
        if command.validate().is_err() {
            return self.reject(
                command.command_id,
                0.0,
                RejectReason::MalformedCommand,
                None,
            );
        }
        if let Some(reason) = self.freshness_refusal(command.expires_at_ms) {
            return self.reject(command.command_id, 0.0, reason, None);
        }
        self.environment_mut().weight.tare();
        let result = CommandResult {
            command_id: command.command_id,
            status: CommandStatus::Completed,
            requested_ml: 0.0,
            delivered_ml: Some(0.0),
            duration_ms: Some(0),
            clamped: false,
            reason: None,
            delivered_today_ml: self.delivered_today_ml(),
            origin: CommandOrigin::EdgeCommand,
            detail: Some(String::from("tare")),
        };
        self.record_and_queue(result)
    }

    /// The one place a dose is decided and begun.
    fn dose(
        &mut self,
        command: WaterCommand,
        origin: CommandOrigin,
        detail: Option<String>,
    ) -> Vec<Publication> {
        // A device that cannot trust its stored safety history must not water,
        // whatever the gate would say: the gate's own dedup and budget inputs
        // are exactly the state that is in doubt. `pump_unavailable` is the
        // truthful reason — this device cannot operate its pump — and the
        // specifics travel in `detail` (protocol §5.10 fixes the reason
        // vocabulary, so a new variant is not available).
        if !self.actuation_permitted() {
            let detail = self
                .persistent_state_fault()
                .map(|fault| format!("persistent state fault: {}", fault.reason));
            return self.reject(
                command.command_id,
                command.requested_ml,
                RejectReason::PumpUnavailable,
                detail,
            );
        }

        // A single-pump device runs one dose at a time. Without this, a
        // redelivery arriving *during* a dose would find nothing in the dedup
        // ring — the outcome is not recorded until the run ends — and would
        // start the pump again on top of the dose already running. The ring
        // protects against a repeat after completion; this protects against a
        // repeat before it.
        if let Some(in_flight) = self.store().state().in_flight_dose.clone() {
            tracing::warn!(
                command_id = %command.command_id,
                in_flight = %in_flight.command_id,
                "a command arrived while a dose was already running; refusing"
            );
            return self.reject(
                command.command_id,
                command.requested_ml,
                RejectReason::PumpUnavailable,
                Some(format!(
                    "a dose for {} is already running",
                    in_flight.command_id
                )),
            );
        }

        match self.decide(&command) {
            Decision::AlreadyExecuted(previous) => {
                tracing::info!(
                    command_id = %command.command_id,
                    "command already executed; republishing the stored result"
                );
                self.queue_result(previous)
            }
            Decision::Reject(reason) => {
                tracing::info!(
                    command_id = %command.command_id,
                    ?reason,
                    "command refused"
                );
                self.reject(command.command_id, command.requested_ml, reason, detail)
            }
            Decision::Accept {
                effective_ml,
                run_ms,
                clamped,
            } => self.begin_dose(command, effective_ml, run_ms, clamped, origin, detail),
        }
    }

    /// Begins a dose the device decided on for itself while isolated.
    ///
    /// The **same** actuation path as a command: the same in-flight NVS write
    /// before the pump moves (SAFETY-011), the same `begin_dose`, the same
    /// `start_pump`. What differs is where the volume came from and what bounds
    /// it: `bound_dose` is protocol §5.8 steps 10-12 extracted verbatim, so the
    /// firmware ceilings apply to autonomous water exactly as they do to
    /// commanded water (SAFETY-007, SAFETY-014).
    ///
    /// Steps 2 and 3 — `clock_unsynced` and `expired` — are deliberately absent.
    /// They are properties of a *command*: a decision another machine made at a
    /// wall-clock instant. An isolated device has no issuer, no TTL, and by
    /// construction no synchronised clock. SAFETY-015 governs this path and
    /// SAFETY-002 governs the commanded one; applying the TTL here would mean an
    /// isolated device could never water, which is the whole feature.
    pub(crate) fn autonomous_dose(&mut self, ml: f32) -> Vec<Publication> {
        if !self.actuation_permitted() {
            return Vec::new();
        }
        if self.store().state().in_flight_dose.is_some() {
            // One pump, one dose. The evaluator is re-run next tick.
            return Vec::new();
        }
        let bound = rhizo_mqtt_contract::safety::bound_dose(
            ml,
            self.config().pump.ml_per_second,
            self.store().state().delivered_today_ml,
        );
        let rhizo_mqtt_contract::safety::DoseBound::Accept {
            effective_ml,
            run_ms,
            clamped,
        } = bound
        else {
            tracing::warn!(
                requested_ml = ml,
                ?bound,
                "an autonomous dose was refused by the firmware hard limits"
            );
            return Vec::new();
        };
        // The id is device-generated and stable, so the dose deduplicates and
        // reconciles like any other and the edge can attribute the result.
        let command = WaterCommand {
            command_id: CommandId::from_uuid(self.next_local_uuid()),
            requested_ml: ml,
            // An isolated device has no wall clock to stamp. These are never
            // re-validated: `begin_dose` does not gate, and the gate that would
            // read them is the one this path deliberately does not run.
            issued_at_ms: UtcMillis(0),
            expires_at_ms: UtcMillis(1),
        };
        self.begin_dose(
            command,
            effective_ml,
            run_ms,
            clamped,
            CommandOrigin::OfflineAutonomous,
            Some(String::from("offline autonomous dose")),
        )
    }

    /// Asks the shared gate.
    ///
    /// **The only call site.** Every input it needs is assembled here and
    /// nowhere else, so there is no second place that could assemble them
    /// differently — which is how a "small" divergence would begin.
    fn decide(&self, command: &WaterCommand) -> Decision {
        let state = self.store().state();
        let previous: Vec<PreviousCommand<'_>> = state
            .command_ring
            .iter()
            .map(|record| PreviousCommand {
                command_id: record.command_id,
                result: &record.result,
            })
            .collect();
        let guard = DeviceGuardState {
            previous: &previous,
            clock_synced: self.clock_synced(),
            now_ms: self.wall_now().unwrap_or(UtcMillis(0)),
            leak: self.leak_input(),
            tank_percent: self.tank_input(),
            tank_min_percent: self.config().tank.min_percent,
            pump_enabled: self.config().pump.enabled
                && self.capabilities().primary_actuator().is_some(),
            pump_faulted: self.actuator_faulted(),
            pump_ml_per_second: self.config().pump.ml_per_second,
            delivered_today_ml: state.delivered_today_ml,
        };
        match validate_water_command(command, &guard) {
            CommandVerdict::Accept {
                effective_ml,
                run_ms,
                clamped,
            } => Decision::Accept {
                effective_ml,
                run_ms,
                clamped,
            },
            CommandVerdict::Reject(reason) => Decision::Reject(reason),
            CommandVerdict::AlreadyExecuted { previous } => {
                Decision::AlreadyExecuted(previous.clone())
            }
        }
    }

    /// Persists the in-flight record, then starts the pump.
    fn begin_dose(
        &mut self,
        command: WaterCommand,
        effective_ml: f32,
        run_ms: u32,
        clamped: bool,
        origin: CommandOrigin,
        detail: Option<String>,
    ) -> Vec<Publication> {
        let in_flight = InFlightDose {
            command_id: command.command_id,
            started_at_ms: self.wall_now(),
            started_at_monotonic_ms: self.uptime_ms(),
            requested_ml: command.requested_ml,
            effective_ml,
        };
        // Step 13 before step 14, always. If the process dies between this
        // write and the pump starting, the next boot reports `interrupted` —
        // which is conservative and correct. If it were the other way round, a
        // death mid-dose would leave no record at all and the same command
        // could run a second time.
        if let Err(e) = self.store_mut().mutate(|state| {
            state.in_flight_dose = Some(in_flight.clone());
        }) {
            tracing::error!(error = %e, "could not persist the in-flight dose; refusing to actuate");
            return self.reject(
                command.command_id,
                command.requested_ml,
                RejectReason::PumpUnavailable,
                Some(String::from("in-flight dose could not be persisted")),
            );
        }
        self.pending_detail = detail;
        self.pending_clamped = clamped;
        self.pending_origin = origin;
        tracing::info!(
            command_id = %command.command_id,
            effective_ml,
            run_ms,
            clamped,
            "dose accepted; pump starting"
        );
        let publications = self.start_pump(command.command_id, run_ms, effective_ml);

        // `restart-mid-dose` kills the device here: **after** the in-flight
        // record reached the store, and with the pump energised. Killing it
        // before the write would leave no record and exercise nothing; killing
        // it after the dose completed would exercise the ordinary path. This
        // instant is the one SAFETY-011 is about.
        if self.faults().is_enabled("restart-mid-dose") {
            tracing::warn!(
                command_id = %command.command_id,
                "restart-mid-dose: terminating during actuation"
            );
            self.disable_fault("restart-mid-dose");
            self.restart();
            // The restarted device reports the interrupted dose from its own
            // boot path, so its publications are the ones that go out.
            return self.flush_results();
        }
        publications
    }

    /// The tri-state leak input.
    ///
    /// A device with no declared leak sensor reports `Unknown`, never `Clear`.
    /// The shared gate refuses `Unknown` (step 6), so such a device cannot
    /// water at all — which is the intended fail-closed reading of "absence is
    /// not permission" (SAFETY-012), not an oversight.
    fn leak_input(&self) -> LeakState {
        if self.samples_kind(&rhizo_mqtt_contract::payload::MeasurementKind::LeakState) {
            self.environment().tank.leak()
        } else {
            LeakState::Unknown
        }
    }

    /// The tank level input, or `None` when the device has no level sensor.
    ///
    /// The *true* level rather than a noisy sample: measurement noise belongs to
    /// what the device reports, not to what it uses to decide, and a threshold
    /// that flipped on noise would make a safety refusal a coin toss.
    fn tank_input(&self) -> Option<f32> {
        self.samples_kind(&rhizo_mqtt_contract::payload::MeasurementKind::TankLevel)
            .then(|| self.environment().tank.true_percent() as f32)
    }

    /// Refuses a command whose freshness cannot be established.
    ///
    /// The same two conditions the water gate checks first, in the same order,
    /// for the commands that do not move water.
    fn freshness_refusal(&self, expires_at_ms: UtcMillis) -> Option<RejectReason> {
        // Order matters, and the obvious spelling is wrong: reading the clock
        // first and propagating its absence with `?` returns "no refusal" for a
        // device that has no clock at all — fail-open, and invisible. The
        // absence of a wall time *is* the refusal.
        if !self.clock_synced() {
            return Some(RejectReason::ClockUnsynced);
        }
        let Some(now) = self.wall_now() else {
            return Some(RejectReason::ClockUnsynced);
        };
        let skew = rhizo_mqtt_contract::safety::MAX_CLOCK_SKEW_SECONDS * 1000;
        (now.0 > expires_at_ms.0.saturating_add(skew)).then_some(RejectReason::Expired)
    }

    /// Builds, records, and queues a rejection.
    ///
    /// A `command.result` is published for **every** command, rejections
    /// included: an edge that hears nothing cannot tell a refusal from a lost
    /// message, and would eventually reissue.
    fn reject(
        &mut self,
        command_id: CommandId,
        requested_ml: f32,
        reason: RejectReason,
        detail: Option<String>,
    ) -> Vec<Publication> {
        let result = CommandResult {
            command_id,
            status: CommandStatus::Rejected,
            requested_ml,
            delivered_ml: None,
            duration_ms: None,
            clamped: false,
            reason: Some(reason),
            delivered_today_ml: self.delivered_today_ml(),
            origin: CommandOrigin::EdgeCommand,
            detail,
        };
        self.record_and_queue(result)
    }

    /// Records an outcome in the dedup ring and queues its result.
    pub(crate) fn record_and_queue(&mut self, result: CommandResult) -> Vec<Publication> {
        let record = CommandRecord {
            command_id: result.command_id,
            result: result.clone(),
        };
        if let Err(e) = self
            .store_mut()
            .mutate(|state| state.record_command(record))
        {
            tracing::error!(error = %e, "could not persist the command outcome");
        }
        self.queue_result(result)
    }

    /// Queues a result for publication, persisting it first.
    ///
    /// Persisted before it is published so a result that cannot be delivered is
    /// republished after the next boot: a result is ledger data, not a sample
    /// (protocol §5.10).
    pub(crate) fn queue_result(&mut self, result: CommandResult) -> Vec<Publication> {
        if let Err(e) = self.store_mut().mutate(|state| {
            state
                .pending_results
                .retain(|r| r.command_id != result.command_id);
            state.pending_results.push(result);
            // The ring is the bound, exactly as it is for buffered events. An
            // isolated device that autonomously waters for a week would
            // otherwise grow this list without limit, and unbounded growth in
            // the one structure that must survive a reboot is worse than losing
            // the oldest transport copy of a dose that is *also* recorded as a
            // `watering.offline_autonomous` audit event.
            while state.pending_results.len() > PENDING_RESULT_LIMIT {
                let dropped = state.pending_results.remove(0);
                tracing::error!(
                    command_id = %dropped.command_id,
                    "the pending-result buffer is full; the oldest unacknowledged result was evicted"
                );
            }
        }) {
            tracing::error!(error = %e, "could not persist a pending result");
        }
        self.flush_results()
    }

    /// Publishes every pending result, **keeping** them until the edge
    /// acknowledges each one.
    ///
    /// # Why handing it to the broker is not delivery
    ///
    /// MQTT QoS 1 acknowledges hop by hop. The PUBACK for this publication is
    /// written by the broker on receipt; the edge may not have read the message,
    /// may crash before its transaction commits, and -- with a clean session --
    /// will never be offered it again. Clearing the entry here, as this used to,
    /// discarded the device's only remaining copy of a result the edge had never
    /// recorded. A lost `completed` under-counts the rolling 24-hour budget
    /// (SAFETY-006), and under-counting is the direction that waters again too
    /// soon.
    ///
    /// So the entry survives until `command.result.ack` names it (protocol
    /// §5.14). Republishing a result the edge already holds costs one message
    /// and is deduplicated on `command_id`; deleting one it never held loses
    /// ledger data for ever. This is the same trade `event.ack` already makes.
    ///
    /// **`clean_session=false` would not be a substitute.** A persistent session
    /// would make the *broker* redeliver to the edge, which helps only while the
    /// broker itself survives, only for messages it has already accepted, and
    /// not at all for the device's own copy. The durability question is between
    /// the device and the edge, so the answer has to be too.
    pub(crate) fn flush_results(&mut self) -> Vec<Publication> {
        if !self.is_connected() {
            return Vec::new();
        }
        let pending = self.store().state().pending_results.clone();
        if pending.is_empty() {
            return Vec::new();
        }
        let topic = Topic::CommandResult(self.device_id().clone());
        let mut publications = Vec::new();
        for result in pending {
            match self.seal_result(topic.clone(), result.clone()) {
                Ok(publication) => publications.push(publication),
                Err(e) => {
                    tracing::error!(error = %e, "could not encode a command result");
                    break;
                }
            }
        }
        self.note_result_publish();
        publications
    }

    /// Republishes unacknowledged results once the retry interval has elapsed.
    ///
    /// Called from the tick. Retrying only on reconnect would leave a result
    /// stranded for as long as the connection happened to hold, which is the
    /// common case: the edge crashes and restarts while the device's socket to
    /// the broker never drops.
    pub(crate) fn retry_unacknowledged_results(&mut self) -> Vec<Publication> {
        if !self.is_connected() || self.store().state().pending_results.is_empty() {
            return Vec::new();
        }
        let now = self.elapsed_ms();
        if now.saturating_sub(self.last_result_publish_ms()) < COMMAND_RESULT_RETRY_MS {
            return Vec::new();
        }
        tracing::debug!(
            pending = self.store().state().pending_results.len(),
            "retrying results the edge has not acknowledged"
        );
        self.flush_results()
    }

    /// Applies a `command.result.ack` (protocol §5.14).
    ///
    /// Deletion is the last step and is one persisted mutation, the same shape
    /// as `event.ack`: a crash between "decided to delete" and "wrote the file"
    /// leaves the result still pending, and a redundant republication is the
    /// cheap failure.
    pub(crate) fn on_command_result_ack(&mut self, payload: &[u8]) -> Vec<Publication> {
        let Some(envelope) = self.decode::<CommandResultAck>(payload, "command.result.ack") else {
            return Vec::new();
        };
        let acked = envelope.data.command_id;
        let held = self
            .store()
            .state()
            .pending_results
            .iter()
            .any(|r| r.command_id == acked);
        if !held {
            // Not an error. An acknowledgement for a result this device has
            // already dropped is the ordinary outcome of a duplicate delivery.
            tracing::debug!(command_id = %acked, "acknowledged a result that is no longer pending");
            return Vec::new();
        }
        if let Err(e) = self
            .store_mut()
            .mutate(|state| state.pending_results.retain(|r| r.command_id != acked))
        {
            tracing::error!(error = %e, "could not persist the acknowledgement; the result is kept");
            return Vec::new();
        }
        tracing::info!(command_id = %acked, "the edge durably committed a result");
        Vec::new()
    }

    /// Reads a stored outcome for a command, if there is one.
    fn stored_result(&self, command_id: CommandId) -> Option<CommandResult> {
        self.store()
            .state()
            .previous(command_id)
            .map(|record| record.result.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::cli;
    use rhizo_mqtt_contract::payload::MeasurementKind;
    use rhizo_mqtt_contract::safety::{FIRMWARE_MAX_DAILY_ML, FIRMWARE_MAX_ML_PER_RUN};
    use rhizo_mqtt_contract::{DeviceId, Envelope, MessageId};
    use uuid::Uuid;

    const SYNCED_AT_MS: i64 = 1_756_121_400_000;

    fn topic(kind: &str) -> Topic {
        let id = DeviceId::parse("plant-node-01").unwrap();
        match kind {
            "water" => Topic::CommandWater(id),
            "tare" => Topic::CommandTare(id),
            "calibrate" => Topic::CommandCalibrate(id),
            other => panic!("no such command topic: {other}"),
        }
    }

    fn time_topic() -> Topic {
        Topic::Time(DeviceId::parse("plant-node-01").unwrap())
    }

    fn command_id(n: u128) -> CommandId {
        CommandId::from_uuid(Uuid::from_u128(n))
    }

    fn envelope(kind: &str, data: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "v": 1,
            "kind": kind,
            "message_id": MessageId::from_uuid(Uuid::from_u128(999)),
            "device_id": "plant-node-01",
            "data": data,
        }))
        .unwrap()
    }

    fn edge_time(ms: i64) -> Vec<u8> {
        envelope("edge.time", serde_json::json!({ "edge_time_ms": ms }))
    }

    fn water(id: u128, requested_ml: f32) -> Vec<u8> {
        water_expiring(id, requested_ml, SYNCED_AT_MS, SYNCED_AT_MS + 120_000)
    }

    fn water_expiring(
        id: u128,
        requested_ml: f32,
        issued_at_ms: i64,
        expires_at_ms: i64,
    ) -> Vec<u8> {
        envelope(
            "command.water",
            serde_json::json!({
                "command_id": command_id(id),
                "requested_ml": requested_ml,
                "issued_at_ms": issued_at_ms,
                "expires_at_ms": expires_at_ms,
            }),
        )
    }

    /// A connected, synchronised device with a full tank and a clear tray.
    fn ready(args: &[&str]) -> Device {
        let mut device = Device::new(&cli(args));
        device.on_connected().unwrap();
        device.on_message(&time_topic(), &edge_time(SYNCED_AT_MS));
        assert!(device.clock_synced(), "the fixture must be synchronised");
        device
    }

    fn results(published: &[Publication]) -> Vec<CommandResult> {
        published
            .iter()
            .filter(|p| matches!(p.topic, Topic::CommandResult(_)))
            .map(|p| {
                Envelope::<CommandResult>::from_json(p.payload.as_bytes())
                    .unwrap()
                    .data
            })
            .collect()
    }

    /// Runs the device forward until the pump has finished whatever it started.
    fn run_to_completion(device: &mut Device) -> Vec<CommandResult> {
        let mut all = Vec::new();
        for _ in 0..1_000 {
            let published = device.tick(100);
            let batch = results(&published);
            acknowledge(device, &batch);
            all.extend(batch);
            if !device.pump_running() {
                break;
            }
        }
        assert!(!device.pump_running(), "the pump must stop on its own");
        all
    }

    /// Plays the edge's half of protocol §5.14 for each published result.
    ///
    /// Since a result is retained until the **edge** acknowledges it, a test
    /// that never acknowledges sees the same result again on the next retry —
    /// which is the behaviour under test in its own case, and pure noise in
    /// every other. Acknowledging is what a live edge does after its commit.
    fn acknowledge(device: &mut Device, batch: &[CommandResult]) {
        for result in batch {
            let payload = envelope(
                "command.result.ack",
                serde_json::json!({ "command_id": result.command_id }),
            );
            device.on_message(&result_ack_topic(), &payload);
        }
    }

    fn result_ack_topic() -> Topic {
        Topic::CommandResultAck(DeviceId::parse("plant-node-01").unwrap())
    }

    // ------------------------------------------------------- the happy path

    #[test]
    fn a_valid_command_runs_the_pump_and_reports_completed() {
        let mut device = ready(&["--initial-moisture", "20"]);
        let tank_before = device.environment().tank.remaining_ml();

        let published = device.on_message(&topic("water"), &water(1, 40.0));
        assert!(
            results(&published).is_empty(),
            "the result comes when the dose finishes, not when it starts"
        );
        assert!(device.pump_running());

        let results = run_to_completion(&mut device);
        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(result.delivered_ml, Some(40.0));
        assert!(!result.clamped);
        assert_eq!(result.reason, None);
        assert_eq!(result.origin, CommandOrigin::EdgeCommand);
        assert_eq!(result.delivered_today_ml, 40.0);
        assert!(
            (device.environment().tank.remaining_ml() - (tank_before - 40.0)).abs() < 1e-9,
            "the reservoir really fell by what was delivered"
        );
    }

    #[test]
    fn the_in_flight_record_is_written_before_the_pump_starts() {
        let mut device = ready(&[]);
        device.on_message(&topic("water"), &water(1, 40.0));
        let in_flight = device
            .store()
            .state()
            .in_flight_dose
            .clone()
            .expect("the dose must be persisted before actuation");
        assert_eq!(in_flight.command_id, command_id(1));
        assert_eq!(in_flight.requested_ml, 40.0);
        assert!(device.pump_running());

        run_to_completion(&mut device);
        assert!(
            device.store().state().in_flight_dose.is_none(),
            "and it is cleared when the dose completes"
        );
    }

    // ------------------------------------------------------ dedup, SAFETY-001

    #[test]
    fn safety_001_duplicate_command_single_actuation() {
        let mut device = ready(&["--initial-moisture", "20"]);
        device.on_message(&topic("water"), &water(1, 40.0));
        let mut all = run_to_completion(&mut device);
        let tank_after_one = device.environment().tank.remaining_ml();

        for _ in 0..2 {
            all.extend(results(
                &device.on_message(&topic("water"), &water(1, 40.0)),
            ));
            assert!(!device.pump_running(), "a repeat must never actuate");
        }
        assert_eq!(all.len(), 3, "three commands, three results");
        assert!(
            all.iter().all(|r| r.status == CommandStatus::Completed),
            "the stored result is republished verbatim"
        );
        assert!(
            all.iter().all(|r| r.delivered_ml == Some(40.0)),
            "including its delivered volume"
        );
        assert_eq!(
            device.environment().tank.remaining_ml(),
            tank_after_one,
            "and exactly one actuation happened"
        );
    }

    #[test]
    fn deduplication_survives_a_restart() {
        let settings = cli(&["--initial-moisture", "20"]);
        {
            let mut device = Device::new(&settings);
            device.on_connected().unwrap();
            device.on_message(&time_topic(), &edge_time(SYNCED_AT_MS));
            device.on_message(&topic("water"), &water(7, 40.0));
            run_to_completion(&mut device);
        }

        // A fresh process, the same state file, the same command.
        let mut device = Device::new(&settings);
        device.on_connected().unwrap();
        device.on_message(&time_topic(), &edge_time(SYNCED_AT_MS + 1));
        let published = device.on_message(&topic("water"), &water(7, 40.0));
        assert!(
            !device.pump_running(),
            "a command the device already ran must not run again after a reboot"
        );
        let republished = results(&published);
        assert!(
            republished
                .iter()
                .any(|r| r.command_id == command_id(7) && r.status == CommandStatus::Completed),
            "the stored result is republished instead"
        );
    }

    // ------------------------------------------------------------ refusals

    #[test]
    fn safety_002_expired_command_rejected() {
        let mut device = ready(&[]);
        // Well-formed on the wire — expiry after issue — but long past its
        // expiry by the time it arrives. A command that is merely malformed
        // would be refused for a different reason and prove nothing about
        // expiry handling.
        let published = device.on_message(
            &topic("water"),
            &water_expiring(1, 40.0, SYNCED_AT_MS - 300_000, SYNCED_AT_MS - 60_000),
        );
        let result = &results(&published)[0];
        assert_eq!(result.status, CommandStatus::Rejected);
        assert_eq!(result.reason, Some(RejectReason::Expired));
        assert!(!device.pump_running());
    }

    #[test]
    fn an_unsynchronised_clock_refuses_everything() {
        let mut device = Device::new(&cli(&[]));
        device.on_connected().unwrap();
        assert!(!device.clock_synced());
        let result = &results(&device.on_message(&topic("water"), &water(1, 40.0)))[0];
        assert_eq!(result.reason, Some(RejectReason::ClockUnsynced));
        assert!(!device.pump_running());
    }

    /// SAFETY-007. The assertion everything M6 claims about the hard limit
    /// rests on: an absurd request is never delivered in full.
    #[test]
    fn safety_007_oversized_command_clamped_not_delivered() {
        let mut device = ready(&["--initial-moisture", "10"]);
        let tank_before = device.environment().tank.remaining_ml();
        device.on_message(&topic("water"), &water(1, 10_000.0));
        let results = run_to_completion(&mut device);
        let result = &results[0];

        assert_eq!(result.status, CommandStatus::Completed);
        assert!(result.clamped, "the device must say the limit changed it");
        let delivered = result.delivered_ml.unwrap();
        assert!(
            delivered <= FIRMWARE_MAX_ML_PER_RUN,
            "{delivered} ml exceeds the compile-time hard limit"
        );
        let drawn = tank_before - device.environment().tank.remaining_ml();
        assert!(
            drawn <= f64::from(FIRMWARE_MAX_ML_PER_RUN) + 1e-9,
            "{drawn} ml actually left the reservoir"
        );
    }

    #[test]
    fn a_detected_leak_refuses_the_dose() {
        let mut device = ready(&[]);
        device.environment_mut().tank.set_leak(LeakState::Detected);
        let result = &results(&device.on_message(&topic("water"), &water(1, 40.0)))[0];
        assert_eq!(result.reason, Some(RejectReason::LeakDetected));
        assert!(!device.pump_running());
    }

    #[test]
    fn a_device_with_no_leak_sensor_cannot_water_at_all() {
        let mut device = ready(&["--sensors", "soil,tank"]);
        assert!(!device.samples_kind(&MeasurementKind::LeakState));
        let result = &results(&device.on_message(&topic("water"), &water(1, 40.0)))[0];
        assert_eq!(
            result.reason,
            Some(RejectReason::LeakUnknown),
            "absence of evidence is not permission (SAFETY-012)"
        );
    }

    #[test]
    fn a_low_tank_refuses_and_a_missing_tank_sensor_refuses_differently() {
        let mut device = ready(&[]);
        device.environment_mut().tank.set_percent(10.0);
        let result = &results(&device.on_message(&topic("water"), &water(1, 40.0)))[0];
        assert_eq!(result.reason, Some(RejectReason::TankLow));

        let mut blind = ready(&["--sensors", "soil,leak"]);
        let result = &results(&blind.on_message(&topic("water"), &water(2, 40.0)))[0];
        assert_eq!(
            result.reason,
            Some(RejectReason::TankUnknown),
            "an unmeasured tank is Unknown, never Low: Low is a measurement"
        );
    }

    #[test]
    fn a_faulted_or_absent_actuator_refuses() {
        let mut device = ready(&[]);
        device.set_actuator_faulted(true);
        let result = &results(&device.on_message(&topic("water"), &water(1, 40.0)))[0];
        assert_eq!(result.reason, Some(RejectReason::PumpUnavailable));

        let mut monitoring_only = ready(&["--actuators", ""]);
        let result = &results(&monitoring_only.on_message(&topic("water"), &water(2, 40.0)))[0];
        assert_eq!(
            result.reason,
            Some(RejectReason::PumpUnavailable),
            "a monitoring-only plant has no actuation path at all"
        );
    }

    #[test]
    fn the_daily_cap_refuses_once_it_would_be_exceeded() {
        let mut device = ready(&["--initial-moisture", "10", "--tank-capacity-ml", "20000"]);
        let mut delivered_total = 0.0f32;
        let mut refusal = None;
        for id in 0..20u128 {
            let published = device.on_message(&topic("water"), &water(id, 80.0));
            let mut all = results(&published);
            acknowledge(&mut device, &all);
            all.extend(run_to_completion(&mut device));
            for result in all {
                match result.status {
                    CommandStatus::Completed => {
                        delivered_total += result.delivered_ml.unwrap_or(0.0);
                    }
                    CommandStatus::Rejected => refusal = result.reason,
                    other => panic!("unexpected status {other:?}"),
                }
            }
            if refusal.is_some() {
                break;
            }
        }
        assert_eq!(refusal, Some(RejectReason::OverDailyMax));
        assert!(
            delivered_total <= FIRMWARE_MAX_DAILY_ML,
            "{delivered_total} ml exceeds the daily hard limit"
        );
    }

    #[test]
    fn a_malformed_command_is_rejected_rather_than_guessed_at() {
        let mut device = ready(&[]);
        for (id, requested) in [(50u128, 0.0f32), (51, -5.0)] {
            let published = device.on_message(&topic("water"), &water(id, requested));
            let result = &results(&published)[0];
            assert_eq!(result.reason, Some(RejectReason::MalformedCommand));
        }
        assert!(!device.pump_running());
    }

    #[test]
    fn a_command_for_another_device_produces_nothing_at_all() {
        let mut device = ready(&[]);
        let foreign = serde_json::to_vec(&serde_json::json!({
            "v": 1,
            "kind": "command.water",
            "message_id": MessageId::from_uuid(Uuid::from_u128(1)),
            "device_id": "plant-node-02",
            "data": {
                "command_id": command_id(1),
                "requested_ml": 40.0,
                "issued_at_ms": SYNCED_AT_MS,
                "expires_at_ms": SYNCED_AT_MS + 120_000,
            },
        }))
        .unwrap();
        assert!(
            device.on_message(&topic("water"), &foreign).is_empty(),
            "a misrouted command is not answered"
        );
        assert!(!device.pump_running());
    }

    // ----------------------------------------------- persistent-state fault

    #[test]
    fn a_persistent_state_fault_refuses_every_dose() {
        let settings = cli(&[]);
        std::fs::write(settings.resolved_state_file(), b"not a state file").unwrap();
        let mut device = Device::new(&settings);
        device.on_connected().unwrap();
        device.on_message(&time_topic(), &edge_time(SYNCED_AT_MS));
        assert!(!device.actuation_permitted());

        let result = &results(&device.on_message(&topic("water"), &water(1, 40.0)))[0];
        assert_eq!(result.status, CommandStatus::Rejected);
        assert_eq!(result.reason, Some(RejectReason::PumpUnavailable));
        assert!(
            result
                .detail
                .as_deref()
                .is_some_and(|d| d.contains("persistent state fault")),
            "the reason vocabulary is fixed, so the specifics travel in `detail`"
        );
        assert!(!device.pump_running());
        // ...and the device is still a working sensor node.
        assert!(!device.tick(10_000).is_empty());
    }

    // ------------------------------------------------- interrupted, SAFETY-011

    #[test]
    fn safety_011_interrupted_dose_reported() {
        let settings = cli(&["--initial-moisture", "20"]);
        {
            let mut device = Device::new(&settings);
            device.on_connected().unwrap();
            device.on_message(&time_topic(), &edge_time(SYNCED_AT_MS));
            device.on_message(&topic("water"), &water(9, 40.0));
            assert!(device.pump_running(), "the dose is in flight");
            // The process dies here: no completion, no result, just a state
            // file with an in-flight record in it.
        }

        let mut device = Device::new(&settings);
        assert!(!device.pump_running(), "a boot begins with the pump off");
        let published = device.on_connected().unwrap();
        let result = results(&published)
            .into_iter()
            .find(|r| r.command_id == command_id(9))
            .expect("the interrupted dose must be reported");
        assert_eq!(result.status, CommandStatus::Interrupted);
        assert_eq!(
            result.delivered_ml, None,
            "the volume is genuinely unknown; a number here would be a fiction"
        );
        assert_eq!(
            result.delivered_today_ml, 40.0,
            "the full requested volume is credited: the device must assume the worst"
        );
        assert!(device.store().state().in_flight_dose.is_none());

        // ...and the same command is now deduplicated rather than re-run.
        device.on_message(&time_topic(), &edge_time(SYNCED_AT_MS + 1));
        device.on_message(&topic("water"), &water(9, 40.0));
        assert!(!device.pump_running());
    }

    // ----------------------------------------------------- result durability

    #[test]
    fn a_result_produced_while_disconnected_is_republished_after_a_reconnect() {
        let mut device = ready(&["--initial-moisture", "20"]);
        device.on_disconnected();
        device.on_message(&topic("water"), &water(1, 40.0));
        let while_offline = run_to_completion(&mut device);
        assert!(
            while_offline.is_empty(),
            "nothing can be published while disconnected"
        );
        assert_eq!(device.store().state().pending_results.len(), 1);

        let published = device.on_connected().unwrap();
        let result = results(&published)
            .into_iter()
            .find(|r| r.command_id == command_id(1))
            .expect("the pending result must be republished");
        assert_eq!(result.status, CommandStatus::Completed);
        // Handing it to the broker is **not** delivery. The broker's PUBACK is
        // hop-by-hop and says nothing about whether the edge committed, so the
        // result is still held (protocol §5.14).
        assert_eq!(
            device.unacknowledged_results(),
            1,
            "a published result is not a delivered one"
        );

        let payload = envelope(
            "command.result.ack",
            serde_json::json!({ "command_id": result.command_id }),
        );
        device.on_message(&result_ack_topic(), &payload);
        assert_eq!(
            device.unacknowledged_results(),
            0,
            "only the edge's own acknowledgement clears a result"
        );
    }

    /// **The durability property, end to end on the device's side.**
    ///
    /// A result is republished for as long as the edge stays silent, and stops
    /// the moment the edge speaks. The failure this covers is an edge that
    /// crashes after the broker's PUBACK and before its commit — the device's
    /// socket never drops, so a retry that only fired on reconnect would never
    /// fire at all.
    #[test]
    fn an_unacknowledged_result_is_retried_until_the_edge_speaks() {
        let mut device = ready(&["--initial-moisture", "20"]);
        device.on_message(&topic("water"), &water(1, 40.0));
        // Deliberately does not acknowledge.
        let mut all = Vec::new();
        for _ in 0..1_000 {
            all.extend(results(&device.tick(100)));
            if !device.pump_running() {
                break;
            }
        }
        assert_eq!(all.len(), 1, "the dose finished once");
        assert_eq!(device.unacknowledged_results(), 1);

        // Silence past the retry interval republishes the same result.
        let retried = results(&device.tick(COMMAND_RESULT_RETRY_MS + 1));
        assert_eq!(retried.len(), 1, "an unacknowledged result is retried");
        assert_eq!(
            retried[0].command_id,
            command_id(1),
            "the retry carries the same command_id, so the edge deduplicates it"
        );

        // Well inside the interval, nothing is republished.
        assert!(
            results(&device.tick(10)).is_empty(),
            "the retry is rate limited, not per tick"
        );

        // The edge commits and says so.
        acknowledge(&mut device, &retried);
        assert_eq!(device.unacknowledged_results(), 0);
        assert!(
            results(&device.tick(COMMAND_RESULT_RETRY_MS + 1)).is_empty(),
            "an acknowledged result is never republished"
        );
    }

    /// An acknowledgement for a result this device is not holding is a no-op,
    /// not an error and never a reason to drop a *different* result.
    #[test]
    fn an_acknowledgement_for_an_unknown_result_changes_nothing() {
        let mut device = ready(&["--initial-moisture", "20"]);
        device.on_message(&topic("water"), &water(1, 40.0));
        for _ in 0..1_000 {
            device.tick(100);
            if !device.pump_running() {
                break;
            }
        }
        assert_eq!(device.unacknowledged_results(), 1);
        let payload = envelope(
            "command.result.ack",
            serde_json::json!({ "command_id": command_id(99) }),
        );
        device.on_message(&result_ack_topic(), &payload);
        assert_eq!(
            device.unacknowledged_results(),
            1,
            "acknowledging one command must never clear another"
        );
    }

    #[test]
    fn a_result_survives_a_restart_and_is_published_on_the_next_boot() {
        let settings = cli(&["--initial-moisture", "20"]);
        {
            let mut device = Device::new(&settings);
            device.on_connected().unwrap();
            device.on_message(&time_topic(), &edge_time(SYNCED_AT_MS));
            device.on_disconnected();
            device.on_message(&topic("water"), &water(3, 40.0));
            run_to_completion(&mut device);
            assert_eq!(device.store().state().pending_results.len(), 1);
        }
        let mut device = Device::new(&settings);
        let published = device.on_connected().unwrap();
        assert!(
            results(&published)
                .iter()
                .any(|r| r.command_id == command_id(3)),
            "a result is ledger data and must outlive the process"
        );
    }

    #[test]
    fn a_result_is_published_for_every_outcome_including_rejections() {
        let mut device = ready(&[]);
        let mut count = 0;
        // One accepted, one rejected, one duplicate.
        let batch = results(&device.on_message(&topic("water"), &water(1, 40.0)));
        acknowledge(&mut device, &batch);
        count += batch.len();
        count += run_to_completion(&mut device).len();
        device.environment_mut().tank.set_leak(LeakState::Detected);
        let batch = results(&device.on_message(&topic("water"), &water(2, 40.0)));
        acknowledge(&mut device, &batch);
        count += batch.len();
        let batch = results(&device.on_message(&topic("water"), &water(1, 40.0)));
        acknowledge(&mut device, &batch);
        count += batch.len();
        assert_eq!(count, 3, "one result per command, always");
    }

    // ------------------------------------------------------- tare, calibrate

    #[test]
    fn a_tare_zeroes_the_scale_and_reports_completed() {
        let mut device = ready(&[]);
        let payload = envelope(
            "command.tare",
            serde_json::json!({
                "command_id": command_id(5),
                "issued_at_ms": SYNCED_AT_MS,
                "expires_at_ms": SYNCED_AT_MS + 60_000,
            }),
        );
        let result = &results(&device.on_message(&topic("tare"), &payload))[0];
        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(result.delivered_ml, Some(0.0));
        assert!(
            !device.pump_running(),
            "taring moves no water and must not start the pump"
        );

        // ...and a repeat is deduplicated like any other command.
        let repeat = &results(&device.on_message(&topic("tare"), &payload))[0];
        assert_eq!(repeat.status, CommandStatus::Completed);
    }

    #[test]
    fn a_tare_with_an_unsynchronised_clock_is_refused() {
        let mut device = Device::new(&cli(&[]));
        device.on_connected().unwrap();
        let payload = envelope(
            "command.tare",
            serde_json::json!({
                "command_id": command_id(5),
                "issued_at_ms": SYNCED_AT_MS,
                "expires_at_ms": SYNCED_AT_MS + 60_000,
            }),
        );
        let result = &results(&device.on_message(&topic("tare"), &payload))[0];
        assert_eq!(result.status, CommandStatus::Rejected);
        assert_eq!(result.reason, Some(RejectReason::ClockUnsynced));
    }

    #[test]
    fn a_calibration_runs_the_pump_and_counts_toward_the_daily_total() {
        let mut device = ready(&["--initial-moisture", "10", "--ml-per-second", "4.0"]);
        let payload = envelope(
            "command.calibrate",
            serde_json::json!({
                "command_id": command_id(6),
                "run_seconds": 5.0,
                "issued_at_ms": SYNCED_AT_MS,
                "expires_at_ms": SYNCED_AT_MS + 60_000,
            }),
        );
        device.on_message(&topic("calibrate"), &payload);
        assert!(device.pump_running());
        let results = run_to_completion(&mut device);
        let result = &results[0];
        assert_eq!(result.status, CommandStatus::Completed);
        assert_eq!(
            result.delivered_ml,
            Some(20.0),
            "five seconds at four millilitres a second"
        );
        assert_eq!(result.detail.as_deref(), Some("calibrate"));
        assert_eq!(
            device.delivered_today_ml(),
            20.0,
            "a calibration's volume counts toward the daily total"
        );
    }

    #[test]
    fn a_calibration_is_subject_to_the_same_hard_limits_as_any_dose() {
        let mut device = ready(&["--initial-moisture", "10", "--ml-per-second", "50.0"]);
        let payload = envelope(
            "command.calibrate",
            serde_json::json!({
                "command_id": command_id(6),
                // 50 ml/s for 10 s is 500 ml: far beyond the per-run limit.
                "run_seconds": 10.0,
                "issued_at_ms": SYNCED_AT_MS,
                "expires_at_ms": SYNCED_AT_MS + 60_000,
            }),
        );
        device.on_message(&topic("calibrate"), &payload);
        let results = run_to_completion(&mut device);
        let delivered = results[0].delivered_ml.unwrap();
        assert!(
            delivered <= FIRMWARE_MAX_ML_PER_RUN,
            "{delivered} ml exceeds the per-run hard limit"
        );
        assert!(results[0].clamped);
    }

    #[test]
    fn a_calibration_with_an_unavailable_pump_is_refused_not_divided_by() {
        let mut device = ready(&[]);
        device.set_actuator_faulted(true);
        let payload = envelope(
            "command.calibrate",
            serde_json::json!({
                "command_id": command_id(6),
                "run_seconds": 5.0,
                "issued_at_ms": SYNCED_AT_MS,
                "expires_at_ms": SYNCED_AT_MS + 60_000,
            }),
        );
        let result = &results(&device.on_message(&topic("calibrate"), &payload))[0];
        assert_eq!(result.reason, Some(RejectReason::PumpUnavailable));
    }

    // ------------------------------------------------------ pump behaviours

    #[test]
    fn a_pump_that_delivers_nothing_reports_a_dose_that_moved_no_water() {
        let mut device = ready(&["--initial-moisture", "20"]);
        device.set_pump_delivers(false);
        let tank_before = device.environment().tank.remaining_ml();
        let weight_before = device.environment().weight.water_g();

        device.on_message(&topic("water"), &water(1, 40.0));
        let results = run_to_completion(&mut device);
        assert_eq!(results[0].status, CommandStatus::Completed);
        assert_eq!(results[0].delivered_ml, Some(0.0));
        assert_eq!(device.environment().tank.remaining_ml(), tank_before);
        let weight_after = device.environment().weight.water_g();
        assert!(
            weight_after <= weight_before,
            "the scale must not rise: that it does not is the signature the edge has to detect"
        );
        assert!(
            weight_before - weight_after < 1.0,
            "and the only change is the evapotranspiration of the seconds that passed"
        );
    }

    #[test]
    fn a_pump_that_will_not_de_energise_is_stopped_by_the_independent_guard() {
        let mut device = ready(&["--initial-moisture", "20"]);
        device.on_message(&topic("water"), &water(1, 40.0));
        device.set_pump_stuck_on(true);
        let results = run_to_completion(&mut device);
        assert_eq!(
            results[0].status,
            CommandStatus::Failed,
            "the run guard stopping the pump is a hardware failure, not a completion"
        );
        assert_eq!(results[0].delivered_ml, None);
        assert!(!device.pump_running());
    }
}
