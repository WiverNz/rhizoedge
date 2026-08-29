//! The process-level run loop.
//!
//! Owns the pieces that only exist in a running process — the broker
//! connection, the tick that converts real elapsed time into virtual time, and
//! the shutdown path — and keeps them out of [`Device`], which stays testable
//! without any of them.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::cli::Cli;
use crate::clock::AcceleratedClock;
use crate::device::Device;
use crate::mqtt::{Connection, MqttError, Step};
use crate::shutdown;

/// How often the run loop converts real elapsed time into virtual time.
///
/// The *sampling* rate of the conversion, not the resolution of the model: a
/// tick's worth of virtual time is applied to the device in bounded steps
/// (`AcceleratedClock::steps`), so the physical model evolves identically at
/// every scale.
pub const TICK_INTERVAL: Duration = Duration::from_millis(100);

/// Runs one device until it is asked to stop.
///
/// # Errors
///
/// Returns a startup failure. Connection failures are not errors: a device
/// reconnects indefinitely (PRD 020 §Failure modes).
pub async fn run(cli: Cli) -> Result<shutdown::Signal, MqttError> {
    let store = crate::state::StateStore::load(cli.resolved_state_file());
    tracing::info!(
        state_file = %store.path().display(),
        boot_count = store.state().boot_count,
        actuation_permitted = store.actuation_permitted(),
        "persistent state loaded"
    );
    let device = Arc::new(Mutex::new(Device::with_store(&cli, store)));
    let mut connection = Connection::new(&cli, Arc::clone(&device))?;

    // The control API runs beside the device, not inside it: a scenario test
    // injects a fault while the run loop keeps ticking. Loopback only, and
    // disabled outright by `--no-control-api`.
    let control_state =
        crate::control::ControlState::new(Arc::clone(&device), Arc::new(cli.clone()));
    let restarted = Arc::clone(&control_state.restarted);
    let control = (!cli.no_control_api).then(|| {
        let state = control_state.clone();
        let port = cli.control_port;
        tokio::spawn(async move {
            if let Err(e) = crate::control::serve(state, port).await {
                tracing::error!(error = %e, port, "the control API could not start");
            }
        })
    });

    // One clock for the whole process. Nothing else reads a clock, so nothing
    // can age at a different rate from the rest of the device (ADR-013).
    let mut clock = AcceleratedClock::new(cli.time_scale);
    tracing::info!(
        time_scale = clock.scale(),
        "virtual time running at this multiple of real time"
    );

    let mut ticker = tokio::time::interval(TICK_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let stopping = Box::pin(stop_signal(cli.duration));
    tokio::pin!(stopping);

    let signal = loop {
        tokio::select! {
            step = connection.step() => {
                if let Step::Disconnected { retry_in } = step {
                    tokio::time::sleep(retry_in).await;
                }
                // The `disconnect` fault holds the device off the broker for a
                // stated duration. Dropping the connection is what makes it a
                // real isolation rather than a flag: the socket closes, the
                // will fires, and nothing can be published until it elapses.
                if crate::mqtt::lock(&device).is_isolated_by_fault() {
                    connection = isolate(&cli, &device, &mut clock, connection).await?;
                }
            }
            _ = ticker.tick() => {
                let (publications, restarted_by_fault) = advance(&device, &mut clock);
                connection.publish_all(&publications).await;
                if restarted_by_fault {
                    rebuild(&cli, &device, &mut connection);
                }
                // The announcement has now gone out. *Then* the socket closes:
                // publishing after disconnecting is not possible, and the
                // ordering is what lets a fresh subscriber see a sleeping device
                // rather than a stale online one (ADR-018 section 5).
                if crate::mqtt::lock(&device).take_sleep_notice() {
                    connection = deep_sleep(&cli, &device, &mut clock, connection).await?;
                }
            }
            () = restarted.notified() => {
                // The device was replaced with a fresh boot. Rebuild the
                // connection so the new `boot_id`, the new will, and a fresh
                // session go to the broker — which is what a restart is.
                rebuild(&cli, &device, &mut connection);
            }
            signal = &mut stopping => break signal,
        }
    };

    connection.shutdown().await;
    if let (Some(capture), Some(directory)) = (connection.capture(), cli.capture_fixtures.as_ref())
    {
        match capture.write_to(directory) {
            Ok(written) => tracing::info!(
                directory = %directory.display(),
                files = written.len(),
                "captured fixtures written"
            ),
            Err(e) => tracing::error!(error = %e, "could not write the captured fixtures"),
        }
    }
    if let Some(control) = control {
        control.abort();
    }
    Ok(signal)
}

/// Holds the device off the broker until the injected isolation elapses.
///
/// The connection is dropped, not merely paused: an isolated device is one whose
/// socket is gone, whose will has fired, and whose telemetry is going into its
/// bounded buffer instead of onto the wire. A "paused" connection would still
/// hold the session open and would test nothing.
async fn isolate(
    cli: &Cli,
    device: &Arc<Mutex<Device>>,
    clock: &mut AcceleratedClock,
    connection: Connection,
) -> Result<Connection, MqttError> {
    drop(connection);
    tracing::warn!(
        remaining_ms = crate::mqtt::lock(device).isolation_remaining_ms(),
        "isolated by an injected fault; sampling and buffering continue"
    );
    loop {
        tokio::time::sleep(TICK_INTERVAL).await;
        // The device keeps running: virtual time advances at the same scale,
        // the model evolves, samples are taken, and everything goes to the ring.
        let _ = advance(device, clock);
        let still_isolated = crate::mqtt::lock(device).is_isolated_by_fault();
        if !still_isolated {
            break;
        }
    }
    tracing::info!("isolation over; reconnecting");
    // A failure here is fatal rather than retryable: `Connection::new` fails
    // only on an unusable broker URL or an unencodable will, both of which
    // already succeeded once at startup. Retrying would loop on a condition
    // that cannot change.
    Connection::new(cli, Arc::clone(device))
}

/// Holds the device off the broker for the announced sleep interval.
///
/// The connection is dropped rather than paused, for the same reason
/// [`isolate`] drops it: a sleeping ESP32 has no socket, and a paused one would
/// keep the session open and test nothing. The plant keeps drying throughout,
/// which is what makes a missed wake visible in the readings that follow it.
async fn deep_sleep(
    cli: &Cli,
    device: &Arc<Mutex<Device>>,
    clock: &mut AcceleratedClock,
    mut connection: Connection,
) -> Result<Connection, MqttError> {
    // A **clean** DISCONNECT, not a dropped socket. A device entering deep sleep
    // leaves deliberately, so the broker must not publish its will: the retained
    // sleep announcement has to survive as the last word on this device, and a
    // will would overwrite it with `connection_lost` and turn an expected
    // absence into an unexplained one. `sleep-without-announcing` is the case
    // that *does* drop the socket, and it does so by publishing nothing first.
    if crate::mqtt::lock(device).sleep_was_announced() {
        connection.disconnect_cleanly().await;
    }
    // An unannounced sleep drops the socket instead, so the broker publishes the
    // will and the absence reads as `connection_lost`.
    drop(connection);
    {
        let mut guard = crate::mqtt::lock(device);
        guard.on_disconnected();
        tracing::info!(
            sleep_ms = guard.power_state().sleep_remaining_ms(),
            "asleep; the radio is off and nothing is published or received"
        );
    }
    loop {
        tokio::time::sleep(TICK_INTERVAL).await;
        let _ = advance(device, clock);
        if !crate::mqtt::lock(device).is_sleeping() {
            break;
        }
    }
    tracing::info!("awake; reconnecting");
    Connection::new(cli, Arc::clone(device))
}

/// Advances the device by the virtual time that has passed.
///
/// The interval is applied in bounded steps rather than as one jump: at
/// `--time-scale 600` a 100 ms tick is a minute of virtual time, and a
/// minute-long step would make the drying curve, the absorption pool, and the
/// overshoot decay each resolve to a single leap. The model would then behave
/// differently at different scales, and an accelerated test would be a test of
/// something other than the system it claims to exercise.
fn advance(
    device: &Arc<Mutex<Device>>,
    clock: &mut AcceleratedClock,
) -> (Vec<crate::envelope::Publication>, bool) {
    let elapsed = clock.take_elapsed_ms();
    let mut publications = Vec::new();
    let mut restarted = false;
    let mut guard = crate::mqtt::lock(device);
    for step in AcceleratedClock::steps(elapsed) {
        publications.extend(guard.tick(step));
        if guard.take_restart_notice() {
            restarted = true;
            // A restart replaced the device; the rest of this interval belongs
            // to the new boot, which starts its own clock at zero.
            break;
        }
    }
    (publications, restarted)
}

/// Replaces the broker connection after a restart.
///
/// A restarted device has a new `boot_id` and a new will; keeping the old
/// socket would publish the old identity and leave the previous will armed.
fn rebuild(cli: &Cli, device: &Arc<Mutex<Device>>, connection: &mut Connection) {
    match Connection::new(cli, Arc::clone(device)) {
        Ok(fresh) => {
            *connection = fresh;
            tracing::warn!("reconnecting after a restart");
        }
        Err(e) => tracing::error!(error = %e, "could not reconnect after a restart"),
    }
}

/// Resolves on a signal or when `--duration` elapses.
async fn stop_signal(duration: Option<u64>) -> shutdown::Signal {
    let signalled = async {
        match shutdown::wait().await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "could not install a signal handler");
                shutdown::Signal::Terminate
            }
        }
    };
    match duration {
        Some(seconds) => tokio::select! {
            s = signalled => s,
            () = tokio::time::sleep(Duration::from_secs(seconds)) => shutdown::Signal::DurationElapsed,
        },
        None => signalled.await,
    }
}
