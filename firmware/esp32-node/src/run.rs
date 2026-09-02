//! The device loop: connect, publish, dispatch, and — in battery mode — sleep.
//!
//! # Where the decisions are not
//!
//! Nothing here decides whether to water, whether a configuration is
//! acceptable, whether a policy may activate, or how much budget is left. Every
//! one of those is a call into `rhizo-node-app`, which has no ESP-IDF
//! dependency and is covered by host tests with fake adapters. This module
//! moves bytes and time.
//!
//! # Ordering that is load-bearing
//!
//! * an interrupted dose is reported **before** anything network-related, and
//!   that already happened in `main` before this function is called;
//! * a `command.result` is published and acknowledged **before** buffered
//!   events are flushed and before sleep is announced, because a result still
//!   in flight when the radio goes down for fifteen minutes turns a completed
//!   dose into an unknown one (M9-021);
//! * the sleep announcement's PUBACK is observed **before** `deep_sleep`
//!   (F-090-52), and sleep is refused while an awake hold is outstanding.

use esp_idf_hal::delay::FreeRtos;
use esp_idf_svc::mqtt::client::EspMqttClient;

use rhizo_mqtt_contract::payload::{
    CommandResult, CommandResultAck, DeviceConfig, DeviceStatus, DeviceStatusValue, EdgeTime,
    EventAck, OfflinePolicySet, PowerMode, PowerStatus, WaterCommand,
};
use rhizo_mqtt_contract::safety::LeakState;
use rhizo_mqtt_contract::{DeviceId, Envelope, MessageKind, Topic, UtcMillis};

use rhizo_node_app::command::{handle_water, GateInputs};
use rhizo_node_app::persist::{BootIdentity, PersistedState};
use rhizo_node_app::policy::UpdateStep;
use rhizo_node_app::ports::{NvsStore, Pump};
use rhizo_node_app::power::{WakeCycle, WakeReason};
use rhizo_node_app::telemetry::{ChemistryCurve, Sensors, TelemetryRing};
use rhizo_node_app::{config, identity, policy, telemetry};

use crate::hal::clock::{monotonic_ms, ClockAt, EdgeClock};
use crate::hal::rng::EspRng;
use crate::net::mqtt::{retain_for, BrokerSettings, QOS};
use crate::net::session::{Inbound, Session};

/// How often the loop wakes to do its housekeeping.
const TICK_MS: u32 = 250;

/// Everything the loop needs that is not a peripheral.
pub struct Context<'a, N: NvsStore, P: Pump> {
    /// Persisted state.
    pub state: &'a mut PersistedState,
    /// Durable storage.
    pub nvs: &'a mut N,
    /// The pump.
    pub pump: &'a mut P,
    /// This boot's identity.
    pub identity: BootIdentity,
    /// The device's own id.
    pub device_id: DeviceId,
    /// Randomness for identifier generation.
    pub rng: EspRng,
}

/// Runs one connected session until it drops.
///
/// Returns when the session ends, so the caller can back off and reconnect —
/// **without limit**, because a device that gives up needs a human.
pub fn serve<N: NvsStore, P: Pump>(
    context: &mut Context<'_, N, P>,
    session: &mut Session,
    clock: &mut EdgeClock,
    cycle: &mut WakeCycle,
    ring: &mut TelemetryRing,
) {
    let mut next_telemetry_ms = 0u64;
    let mut telemetry_cycles: u32 = 0;
    let mut last_unsynced_status_ms = 0u64;
    let mut connected = false;

    loop {
        let now_mono = monotonic_ms();
        cycle.sync_holds();

        for event in session.drain() {
            match event {
                Inbound::Connected => {
                    connected = true;
                    publish_online_status(context, session, clock, cycle, now_mono);
                }
                Inbound::Disconnected => {
                    // The session is gone. Return so the caller reconnects; the
                    // pump is untouched and buffered work is still buffered.
                    log::warn!("mqtt session dropped");
                    return;
                }
                Inbound::Message { topic, payload } => {
                    if let Some(topic) = topic {
                        dispatch(context, session, clock, cycle, &topic, &payload, now_mono);
                    }
                }
                // Logged and otherwise ignored. A broker PUBACK is not
                // delivery: only `command.result.ack` retires a result and only
                // `event.ack` retires history.
                Inbound::PublishAcknowledged(id) => {
                    log::debug!("broker acknowledged publish {id} (not delivery)");
                }
            }
        }

        if !connected {
            FreeRtos::delay_ms(TICK_MS);
            continue;
        }

        // Telemetry on its own cadence, with a status heartbeat every fifth.
        let interval_ms = u64::from(config::telemetry_interval_seconds(context.state)) * 1000;
        if now_mono >= next_telemetry_ms {
            next_telemetry_ms = now_mono.saturating_add(interval_ms);
            publish_telemetry(context, session, clock, ring, now_mono);
            telemetry_cycles = telemetry_cycles.wrapping_add(1);
            if telemetry_cycles.is_multiple_of(telemetry::STATUS_HEARTBEAT_INTERVALS) {
                publish_online_status(context, session, clock, cycle, now_mono);
            }
        }

        // Protocol §5.12: while unsynchronised the retained status *is* the
        // request for `edge.time`, at most once a minute.
        if !clock.is_synced(now_mono)
            && now_mono.saturating_sub(last_unsynced_status_ms)
                >= telemetry::UNSYNCED_STATUS_REPUBLISH_MS
        {
            last_unsynced_status_ms = now_mono;
            publish_online_status(context, session, clock, cycle, now_mono);
        }

        republish_due_results(context, session, clock, now_mono);
        replay_history(context, session, clock, now_mono);

        // Battery mode: once the idle budget is spent and nothing holds the
        // device awake, announce and sleep. The hold is what makes an active
        // watering cycle immune to this.
        if cycle.mode() == PowerMode::Battery
            && cycle.idle_budget_spent(now_mono, config::awake_budget_seconds(context.state))
        {
            announce_and_sleep(context, session, clock, cycle, now_mono);
        }

        FreeRtos::delay_ms(TICK_MS);
    }
}

/// Routes one inbound message to the application logic that owns it.
fn dispatch<N: NvsStore, P: Pump>(
    context: &mut Context<'_, N, P>,
    session: &mut Session,
    clock: &mut EdgeClock,
    cycle: &mut WakeCycle,
    topic: &Topic,
    payload: &[u8],
    now_mono: u64,
) {
    match topic {
        Topic::Time(_) => {
            if let Ok(envelope) = Envelope::<EdgeTime>::from_json(payload) {
                if clock.apply(envelope.data.edge_time_ms, now_mono) {
                    log::info!("clock synchronised from the edge");
                    publish_online_status(context, session, clock, cycle, now_mono);
                }
                // An ignored `edge.time` updates nothing at all — not the
                // clock, not the last applied value, and not the validity
                // window. The shared `TimeSyncState` is what guarantees that.
            }
        }
        Topic::Config(_) => {
            if let Ok(envelope) = Envelope::<DeviceConfig>::from_json(payload) {
                let outcome = config::apply(context.state, &envelope.data);
                log::info!("config: {outcome:?}");
                let _ = context.nvs.store(context.state);
                publish_online_status(context, session, clock, cycle, now_mono);
            }
        }
        Topic::Policy(_) => {
            if let Ok(envelope) = Envelope::<OfflinePolicySet>::from_json(payload) {
                let outcome = policy::apply(context.state, &envelope.data, UpdateStep::Complete);
                log::info!("policy: {outcome:?}");
                let _ = context.nvs.store(context.state);
                publish_online_status(context, session, clock, cycle, now_mono);
            }
        }
        Topic::CommandWater(_) => {
            if let Ok(envelope) = Envelope::<WaterCommand>::from_json(payload) {
                // The hold is acquired before actuation and released when the
                // guard drops, so no error path can leave a battery device
                // awake for ever — or asleep mid-dose.
                let hold = cycle.acquire_hold();
                let inputs = gate_inputs(context, clock, now_mono);
                let handled = handle_water(
                    context.state,
                    context.nvs,
                    context.pump,
                    &envelope.data,
                    &inputs,
                );
                if handled.saturation_raised {
                    log::error!(
                        "pending-result ledger saturated: actuation refused until the edge \
                         acknowledges results ({} ml unaccounted)",
                        context.state.pending_results.unacknowledged_ml()
                    );
                }
                publish_result(context, session, clock, &handled.result, now_mono);
                drop(hold);
                cycle.sync_holds();
            }
        }
        Topic::CommandResultAck(_) => {
            if let Ok(envelope) = Envelope::<CommandResultAck>::from_json(payload) {
                let cleared = rhizo_node_app::command::acknowledge_result(
                    context.state,
                    envelope.data.command_id,
                );
                if cleared {
                    log::info!("pending-result ledger recovered; actuation permitted again");
                }
                let _ = context.nvs.store(context.state);
            }
        }
        Topic::EventsAck(_) => {
            if let Ok(envelope) = Envelope::<EventAck>::from_json(payload) {
                let outcome = context.state.buffer.acknowledge(
                    envelope.data.boot_id,
                    context.identity.boot_id,
                    envelope.data.through_device_seq,
                );
                log::info!("event.ack: {outcome:?}");
                let _ = context.nvs.store(context.state);
            }
        }
        // `tare` and `calibrate` are subscribed to so the subscription set is
        // the normative eight, and are answered in M11 when there is a scale
        // and a calibrated pump to answer for. Receiving and not executing is
        // the conservative behaviour: an unimplemented command is not an
        // executed one.
        Topic::CommandTare(_) | Topic::CommandCalibrate(_) => {
            log::info!("received {topic:?}; not implemented before M11");
        }
        // Never subscribed to: the device publishes all of these. Since the
        // subscriptions became exact topics the broker cannot deliver one here
        // at all, and refusing again costs nothing.
        Topic::Telemetry(_)
        | Topic::Actuator(_)
        | Topic::Events(_)
        | Topic::Status(_)
        | Topic::CommandResult(_) => {}
    }
}

/// The gate inputs, read once so every check in one pass sees the same instant.
fn gate_inputs<N: NvsStore, P: Pump>(
    context: &Context<'_, N, P>,
    clock: &EdgeClock,
    now_mono: u64,
) -> GateInputs {
    let config = context.state.config;
    let at = ClockAt::capture(clock, now_mono);
    let now_ms = rhizo_node_app::ports::Clock::now_ms(&at);
    GateInputs {
        clock_synced: now_ms.is_some(),
        now_ms: UtcMillis(now_ms.unwrap_or(0)),
        // M9 ships fake sensors (PRD 090 §Goals 6), so there is no leak sensor
        // to read yet and the shared gate refuses `Unknown`. That is
        // fail-closed and correct: a device with no leak sensor cannot water,
        // and M10 is what fits one.
        leak: LeakState::Unknown,
        tank_percent: None,
        tank_min_percent: config.map_or(15.0, |c| c.tank.min_percent),
        pump_enabled: config.is_some_and(|c| c.pump.enabled),
        pump_faulted: context.pump.is_faulted(),
        pump_ml_per_second: config.map_or(0.0, |c| c.pump.ml_per_second),
    }
}

/// Publishes on a topic with the retention and QoS the topic requires.
fn publish<T: serde::Serialize>(
    client: &mut EspMqttClient<'static>,
    topic: &Topic,
    envelope: &Envelope<T>,
) {
    let Ok(payload) = envelope.to_json() else {
        log::error!("failed to encode a {:?} payload", envelope.kind);
        return;
    };
    if let Err(error) = client.publish(
        &topic.as_string(),
        QOS,
        retain_for(topic),
        payload.as_bytes(),
    ) {
        log::warn!("publish to {topic:?} failed: {error}");
    }
}

/// Wraps a payload in the envelope every message carries.
fn envelope_for<T, N: NvsStore, P: Pump>(
    context: &mut Context<'_, N, P>,
    clock: &EdgeClock,
    kind: MessageKind,
    now_mono: u64,
    data: T,
) -> Envelope<T> {
    let now_ms = clock.now_ms_at(now_mono).map(UtcMillis);
    // v7 when synchronised, v4 when not: a v7 identifier embeds a timestamp,
    // and minting one from an unsynchronised clock would put a fabricated
    // instant into an id the edge sorts by.
    let message_id = identity::mint_message_id(now_ms, &mut context.rng);
    Envelope {
        v: rhizo_mqtt_contract::PROTOCOL_VERSION,
        kind,
        message_id,
        device_id: context.device_id.clone(),
        boot_id: Some(context.identity.boot_id),
        sequence: Some(now_mono),
        device_time_ms: now_ms,
        clock_synced: Some(now_ms.is_some()),
        data,
    }
}

/// The retained `device.status`.
fn status_for<N: NvsStore, P: Pump>(
    context: &Context<'_, N, P>,
    cycle: &WakeCycle,
    now_mono: u64,
    value: DeviceStatusValue,
    reason: Option<String>,
) -> DeviceStatus {
    DeviceStatus {
        boot_generation: context.identity.boot_generation,
        status: value,
        reason,
        firmware_version: Some(rhizo_node_app::FIRMWARE_VERSION.to_owned()),
        protocol_version: Some(rhizo_mqtt_contract::PROTOCOL_VERSION),
        applied_config_version: context.state.config_version,
        uptime_ms: Some(now_mono),
        free_heap_bytes: Some(free_heap_bytes()),
        rssi_dbm: None,
        applied_policy_versions: policy::applied_versions(context.state),
        connectivity: None,
        power: Some(Box::new(PowerStatus {
            mode: cycle.mode(),
            wake_interval_seconds: (cycle.mode() == PowerMode::Battery)
                .then(|| config::wake_interval_seconds(context.state)),
            expected_wake_ms: None,
            wake_reason: Some(cycle.wake_reason()),
            battery_mv: None,
            awake_ms: Some(cycle.awake_ms(now_mono)),
        })),
        capabilities: rhizo_mqtt_contract::payload::DeviceCapabilities::default(),
        limits: Some(rhizo_node_app::reported_limits()),
    }
}

fn publish_online_status<N: NvsStore, P: Pump>(
    context: &mut Context<'_, N, P>,
    session: &mut Session,
    clock: &EdgeClock,
    cycle: &WakeCycle,
    now_mono: u64,
) {
    let status = status_for(context, cycle, now_mono, DeviceStatusValue::Online, None);
    let envelope = envelope_for(context, clock, MessageKind::DeviceStatus, now_mono, status);
    let topic = Topic::Status(context.device_id.clone());
    publish(session.client(), &topic, &envelope);
}

fn publish_telemetry<N: NvsStore, P: Pump>(
    context: &mut Context<'_, N, P>,
    session: &mut Session,
    clock: &EdgeClock,
    ring: &mut TelemetryRing,
    now_mono: u64,
) {
    // M9 samples through the trait boundary with no sensors fitted, so the
    // batch is empty and nothing is published. M10 fits the probe; the code
    // path, the ring, and the envelope are exercised by the host tests.
    let mut sensors = Sensors {
        soil: None,
        tank: None,
        leak: None,
        scale: None,
        battery: None,
    };
    let curve: Option<ChemistryCurve> = None;
    let batch_id = uuid::Uuid::from_u64_pair(now_mono, context.identity.boot_generation);
    let (batch, errors) = telemetry::sample(batch_id, &mut sensors, curve);
    if errors.total() > 0 {
        log::warn!("sensor errors this cycle: {errors:?}");
    }
    if batch.validate().is_err() {
        return;
    }
    ring.push(batch);
    for batch in ring.drain() {
        let envelope = envelope_for(context, clock, MessageKind::TelemetryBatch, now_mono, batch);
        let topic = Topic::Telemetry(context.device_id.clone());
        publish(session.client(), &topic, &envelope);
    }
}

fn publish_result<N: NvsStore, P: Pump>(
    context: &mut Context<'_, N, P>,
    session: &mut Session,
    clock: &EdgeClock,
    result: &CommandResult,
    now_mono: u64,
) {
    let envelope = envelope_for(
        context,
        clock,
        MessageKind::CommandResult,
        now_mono,
        result.clone(),
    );
    let topic = Topic::CommandResult(context.device_id.clone());
    publish(session.client(), &topic, &envelope);
    context
        .state
        .pending_results
        .mark_published(result.command_id, now_mono);
    let _ = context.nvs.store(context.state);
}

/// Republishes every unacknowledged result whose retry interval has elapsed.
///
/// On a timer, not only on reconnect: the failure this covers is an edge that
/// crashes and restarts while the device's socket never drops (protocol §5.14).
fn republish_due_results<N: NvsStore, P: Pump>(
    context: &mut Context<'_, N, P>,
    session: &mut Session,
    clock: &EdgeClock,
    now_mono: u64,
) {
    let due: Vec<CommandResult> = context
        .state
        .pending_results
        .due(now_mono)
        .into_iter()
        .cloned()
        .collect();
    for result in due {
        publish_result(context, session, clock, &result, now_mono);
    }
}

/// Replays buffered history, sealing any pending gap first.
fn replay_history<N: NvsStore, P: Pump>(
    context: &mut Context<'_, N, P>,
    session: &mut Session,
    clock: &EdgeClock,
    now_mono: u64,
) {
    if context.state.buffer.is_empty() {
        return;
    }
    // Sealed immediately before the replay is built, which is what fixes its
    // range and gives it a sequence above anything the edge can have
    // acknowledged.
    context.state.buffer.seal_gap();
    for batch in context
        .state
        .buffer
        .replay_batches(rhizo_node_app::buffer::REPLAY_BATCH)
    {
        let envelope = envelope_for(context, clock, MessageKind::DeviceEvents, now_mono, batch);
        let topic = Topic::Events(context.device_id.clone());
        publish(session.client(), &topic, &envelope);
    }
    let _ = context.nvs.store(context.state);
}

/// Announces sleep, waits for its acknowledgement, and sleeps.
///
/// Returns only if sleep was refused. The refusals are the whole of "the device
/// does not sleep with a pump running or a result unreported".
fn announce_and_sleep<N: NvsStore, P: Pump>(
    context: &mut Context<'_, N, P>,
    session: &mut Session,
    clock: &EdgeClock,
    cycle: &mut WakeCycle,
    now_mono: u64,
) {
    let status = status_for(
        context,
        cycle,
        now_mono,
        DeviceStatusValue::Offline,
        Some("sleeping".to_owned()),
    );
    // A sleep announcement that would not open a wake window is not an
    // announcement — the edge would read it as an unexplained absence. Better
    // to stay awake than to disappear unannounced.
    if status.announced_sleep_interval_seconds().is_none() {
        log::warn!("sleep announcement would not open a wake window; staying awake");
        return;
    }
    let envelope = envelope_for(context, clock, MessageKind::DeviceStatus, now_mono, status);
    let topic = Topic::Status(context.device_id.clone());
    publish(session.client(), &topic, &envelope);

    // F-090-52: the PUBACK is observed before sleep is entered. The broker's
    // acknowledgement is enough *here* — unlike a `command.result`, a sleep
    // announcement is a retained message the broker will serve to the edge
    // whenever it asks, so the broker having it is the guarantee that matters.
    let deadline = now_mono.saturating_add(5_000);
    while monotonic_ms() < deadline {
        if session
            .drain()
            .iter()
            .any(|event| matches!(event, Inbound::PublishAcknowledged(_)))
        {
            cycle.announcement_acknowledged();
            break;
        }
        FreeRtos::delay_ms(50);
    }

    // Persist before sleeping: RTC memory does not survive a power cut, and the
    // budget floor must.
    let _ = context.nvs.store(context.state);

    match cycle.request_sleep() {
        Ok(()) => {
            let action = cycle.step(monotonic_ms(), 0);
            let interval = config::wake_interval_seconds(context.state);
            let _ = crate::hal::sleep::enter(
                action,
                interval,
                context.identity.boot_generation,
                context.state.offline_runtime.cooldown_remaining_ms,
            );
        }
        Err(refused) => log::info!("sleep refused: {refused:?}"),
    }
}

/// Free heap, reported in status.
fn free_heap_bytes() -> u32 {
    // SAFETY: reads an allocator counter; no preconditions.
    unsafe { esp_idf_sys::esp_get_free_heap_size() }
}

/// Assembles the broker settings from provisioned credentials.
#[must_use]
pub fn broker_url(host: &str) -> String {
    if host.contains("://") {
        host.to_owned()
    } else {
        format!("mqtt://{host}:1883")
    }
}

/// Builds the settings the session needs, or `None` when unprovisioned.
#[must_use]
pub fn settings<'a>(
    state: &'a PersistedState,
    url: &'a str,
    device_id: &'a DeviceId,
) -> BrokerSettings<'a> {
    BrokerSettings {
        url,
        device_id,
        username: state.provisioning.mqtt_user.as_deref(),
        password: state.provisioning.mqtt_pass.as_deref(),
    }
}

/// The wake cycle this boot runs under.
#[must_use]
pub fn wake_cycle(state: &PersistedState, wake_reason: WakeReason) -> WakeCycle {
    WakeCycle::new(config::power_mode(state), wake_reason, monotonic_ms())
}
