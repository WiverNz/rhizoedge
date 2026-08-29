//! The simulator-only control API.
//!
//! # This has no firmware counterpart, and must not acquire one
//!
//! Real devices do not have an HTTP control surface. This exists so a scenario
//! test can flood a tray, empty a reservoir, or freeze a sensor **mid-run**,
//! without restarting the process and without waiting for the condition to
//! occur naturally. It is a test affordance, and the risk it carries is that a
//! future reader infers devices have one — hence this module, this name, and
//! this paragraph.
//!
//! # It is not a second command path
//!
//! Nothing here can deliver a dose. There is no watering endpoint, and
//! `POST /sim/state` reaches the **environment** — soil, reservoir, tray — not
//! the pump. Every route below can only make the device's situation worse or
//! change what it observes; none can make the shared gate say yes. That is the
//! property that keeps `safety_007_simulator_refuses_like_hardware` meaningful
//! while this file exists.
//!
//! # Feature gating: decided against
//!
//! M2-009 asks for a decision. The API is **not** feature-gated out of release
//! builds, and is disabled at runtime by `--no-control-api` instead. A feature
//! gate would produce two builds of the component whose entire job is to be the
//! reference device, and a scenario suite run against the gated-out build would
//! lose fault injection silently — a green suite that tested less than it
//! claimed. Binding to loopback is the containment that actually matters, and
//! that is unconditional.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::cli::{Cli, Fault};
use crate::device::Device;
use crate::mqtt::lock;

/// Everything the control routes need.
#[derive(Clone)]
pub struct ControlState {
    /// The device under control.
    pub device: Arc<Mutex<Device>>,
    /// The settings it was started with, for `POST /sim/restart`.
    pub cli: Arc<Cli>,
    /// Signalled when the device has been restarted, so the run loop rebuilds
    /// the broker connection.
    ///
    /// Without it a restarted device would keep the *old* connection: the old
    /// will, the old client, and a device core that believes it is offline and
    /// never publishes again. A restart that leaves the socket behind is not a
    /// restart, and `--fault restart-mid-dose` would then test a state no real
    /// device can reach.
    pub restarted: Arc<tokio::sync::Notify>,
}

impl ControlState {
    /// Builds the control state for a device.
    #[must_use]
    pub fn new(device: Arc<Mutex<Device>>, cli: Arc<Cli>) -> Self {
        Self {
            device,
            cli,
            restarted: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

/// Builds the router.
pub fn router(state: ControlState) -> Router {
    Router::new()
        .route("/sim/fault", post(set_fault))
        .route("/sim/state", get(get_state).post(set_state))
        .route("/sim/restart", post(restart))
        .route("/sim/scale", get(get_scale))
        .with_state(state)
}

/// The address the control API binds to.
///
/// Loopback, always. A test affordance that accepted connections from the
/// network would be a way to interfere with a device from off the machine, and
/// the simulator is routinely run beside real services.
#[must_use]
pub const fn bind_address(port: u16) -> SocketAddr {
    SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

/// Serves the control API until the process ends.
///
/// # Errors
///
/// Returns a bind or serve failure.
pub async fn serve(state: ControlState, port: u16) -> std::io::Result<()> {
    let address = bind_address(port);
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(
        %address,
        "simulator control API listening (test affordance; no firmware counterpart)"
    );
    axum::serve(listener, router(state)).await
}

// ------------------------------------------------------------------ routes

/// `POST /sim/fault` — `{ "fault": "leak", "enabled": true }`.
#[derive(Debug, Deserialize)]
pub struct FaultRequest {
    /// A fault specification, exactly as `--fault` accepts it.
    pub fault: String,
    /// Whether to enable or disable it.
    #[serde(default = "yes")]
    pub enabled: bool,
}

const fn yes() -> bool {
    true
}

/// What the device is doing, as far as the control API can see.
#[derive(Debug, Serialize)]
pub struct StateResponse {
    /// Device identity.
    pub device_id: String,
    /// Milliseconds since boot.
    pub uptime_ms: u64,
    /// Whether the broker is reachable.
    pub connected: bool,
    /// Whether the wall clock is trustworthy.
    pub clock_synced: bool,
    /// Whether actuation is permitted at all.
    pub actuation_permitted: bool,
    /// The persistent-state fault, if any.
    pub persistent_state_fault: Option<String>,
    /// Soil moisture the soil actually holds.
    pub moisture_vwc: f64,
    /// Moisture delivered but not yet absorbed.
    pub pending_absorption_vwc: f64,
    /// Soil temperature.
    pub temperature_c: f64,
    /// Water mass in the pot.
    pub pot_water_g: f64,
    /// Reservoir level.
    pub tank_percent: f64,
    /// The tri-state leak sensor.
    pub leak: String,
    /// Volume delivered against the daily cap.
    pub delivered_today_ml: f32,
    /// Whether the pump is energised.
    pub pump_running: bool,
    /// Cycles buffered while disconnected.
    pub buffered_cycles: usize,
    /// The declared power mode, `always_on` or `battery`.
    pub power_mode: String,
    /// Whether the device is currently off the air.
    pub sleeping: bool,
    /// Simulated state of charge.
    pub battery_percent: f64,
    /// Faults currently enabled.
    pub faults: Vec<String>,
    /// How many times the device has booted.
    pub boot_count: u64,
}

/// `POST /sim/state` — every field optional; absent fields are left alone.
///
/// Reaches the **environment**, never the pump. There is deliberately no field
/// that starts a dose.
#[derive(Debug, Default, Deserialize)]
pub struct SetStateRequest {
    /// Set soil moisture directly.
    pub moisture_vwc: Option<f64>,
    /// Set the reservoir level.
    pub tank_percent: Option<f64>,
    /// Set the tray leak sensor: `clear`, `detected`, or `unknown`.
    pub leak: Option<String>,
    /// Set the water mass in the pot.
    pub pot_water_g: Option<f64>,
    /// Apply a fertilisation event of this magnitude.
    pub fertilise_us_cm: Option<f64>,
    /// Mark the actuator faulted or healthy.
    pub actuator_faulted: Option<bool>,
    /// Set the simulated state of charge, for a test that needs a low battery
    /// now rather than in a fortnight. Telemetry only: it changes no decision
    /// anywhere (ADR-018 section 7).
    pub battery_percent: Option<f64>,
}

/// `GET /sim/scale`.
#[derive(Debug, Serialize)]
pub struct ScaleResponse {
    /// The configured virtual-time acceleration factor.
    pub time_scale: f64,
}

/// Why a control request was refused.
#[derive(Debug, Serialize)]
pub struct ControlError {
    /// Human-readable reason.
    pub error: String,
}

impl ControlError {
    fn bad_request(message: impl Into<String>) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(Self {
                error: message.into(),
            }),
        )
            .into_response()
    }
}

async fn set_fault(
    State(state): State<ControlState>,
    Json(request): Json<FaultRequest>,
) -> Response {
    let fault: Fault = match request.fault.parse() {
        Ok(fault) => fault,
        Err(e) => return ControlError::bad_request(e.to_string()),
    };
    let mut device = lock(&state.device);
    if request.enabled {
        device.enable_fault(fault);
    } else {
        device.disable_fault(fault.name());
    }
    tracing::warn!(fault = %fault, enabled = request.enabled, "fault injected at runtime");
    Json(snapshot(&device)).into_response()
}

async fn get_state(State(state): State<ControlState>) -> Response {
    Json(snapshot(&lock(&state.device))).into_response()
}

async fn set_state(
    State(state): State<ControlState>,
    Json(request): Json<SetStateRequest>,
) -> Response {
    let leak = match request.leak.as_deref().map(parse_leak) {
        Some(Ok(leak)) => Some(leak),
        Some(Err(e)) => return ControlError::bad_request(e),
        None => None,
    };
    let mut device = lock(&state.device);
    if let Some(vwc) = request.moisture_vwc {
        device.environment_mut().soil.set_vwc(vwc);
    }
    if let Some(percent) = request.tank_percent {
        device.environment_mut().tank.set_percent(percent);
    }
    if let Some(leak) = leak {
        device.environment_mut().tank.set_leak(leak);
    }
    if let Some(grams) = request.pot_water_g {
        device.environment_mut().weight.set_water_g(grams);
    }
    if let Some(us_cm) = request.fertilise_us_cm {
        device.environment_mut().ec.fertilise(us_cm);
    }
    if let Some(percent) = request.battery_percent {
        device.set_battery_percent(percent);
    }
    if let Some(faulted) = request.actuator_faulted {
        device.set_actuator_faulted(faulted);
    }
    Json(snapshot(&device)).into_response()
}

async fn restart(State(state): State<ControlState>) -> Response {
    let mut device = lock(&state.device);
    // `Device::restart` rather than a fresh `Device::new`: the device reboots,
    // the *plant* does not. Rebuilding from settings would reset the soil and
    // the reservoir to their starting values, which is not what happens when a
    // controller loses power beside a pot of wet soil.
    device.restart();
    let _ = device.take_restart_notice();
    tracing::warn!(
        boot_count = device.store().state().boot_count,
        "device restarted through the control API"
    );
    let response = Json(snapshot(&device)).into_response();
    drop(device);
    state.restarted.notify_one();
    response
}

async fn get_scale(State(state): State<ControlState>) -> Response {
    Json(ScaleResponse {
        time_scale: state.cli.time_scale,
    })
    .into_response()
}

fn parse_leak(value: &str) -> Result<rhizo_mqtt_contract::safety::LeakState, String> {
    use rhizo_mqtt_contract::safety::LeakState;
    match value {
        "clear" => Ok(LeakState::Clear),
        "detected" => Ok(LeakState::Detected),
        "unknown" => Ok(LeakState::Unknown),
        other => Err(format!(
            "unknown leak state `{other}`; expected clear, detected, or unknown"
        )),
    }
}

/// Builds the state snapshot every route answers with.
fn snapshot(device: &Device) -> StateResponse {
    use rhizo_mqtt_contract::safety::LeakState;
    let environment = device.environment();
    StateResponse {
        device_id: device.device_id().to_string(),
        uptime_ms: device.uptime_ms(),
        connected: device.is_connected(),
        clock_synced: device.clock_synced(),
        actuation_permitted: device.actuation_permitted(),
        persistent_state_fault: device
            .persistent_state_fault()
            .map(|fault| fault.reason.clone()),
        moisture_vwc: environment.soil.true_vwc(),
        pending_absorption_vwc: environment.soil.pending_vwc(),
        temperature_c: environment.soil.true_temperature_c(),
        pot_water_g: environment.weight.water_g(),
        tank_percent: environment.tank.true_percent(),
        leak: match environment.tank.leak() {
            LeakState::Clear => "clear",
            LeakState::Detected => "detected",
            LeakState::Unknown => "unknown",
        }
        .to_owned(),
        delivered_today_ml: device.delivered_today_ml(),
        pump_running: device.pump_running(),
        buffered_cycles: device.buffered_cycles(),
        power_mode: if device.power_state().is_battery() {
            "battery".to_owned()
        } else {
            "always_on".to_owned()
        },
        sleeping: device.is_sleeping(),
        battery_percent: device.battery_percent(),
        faults: device.faults().active().map(|f| f.to_string()).collect(),
        boot_count: device.store().state().boot_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::cli;

    fn state(args: &[&str]) -> ControlState {
        let settings = cli(args);
        ControlState::new(
            Arc::new(Mutex::new(Device::new(&settings))),
            Arc::new(settings),
        )
    }

    /// Drives one request through the router, returning the status and body.
    async fn request(
        state: &ControlState,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        use tower::ServiceExt;
        let builder = axum::http::Request::builder().method(method).uri(path);
        let request = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
            None => builder.body(axum::body::Body::empty()).unwrap(),
        };
        let response = router(state.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    #[tokio::test]
    async fn the_api_binds_to_loopback_only() {
        let address = bind_address(9090);
        assert!(
            address.ip().is_loopback(),
            "a test affordance must not accept connections from the network"
        );
        assert_eq!(address.port(), 9090);
    }

    #[tokio::test]
    async fn state_can_be_read_back_after_being_set() {
        let state = state(&["--initial-moisture", "42"]);
        let (status, before) = request(&state, "GET", "/sim/state", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(before["moisture_vwc"], 42.0);
        assert_eq!(before["leak"], "clear");

        let (status, after) = request(
            &state,
            "POST",
            "/sim/state",
            Some(serde_json::json!({ "moisture_vwc": 20.0, "tank_percent": 5.0, "leak": "detected" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(after["moisture_vwc"], 20.0);
        assert_eq!(after["tank_percent"], 5.0);
        assert_eq!(after["leak"], "detected");

        let (_, read_back) = request(&state, "GET", "/sim/state", None).await;
        assert_eq!(read_back["moisture_vwc"], 20.0);
    }

    #[tokio::test]
    async fn absent_fields_are_left_alone_rather_than_zeroed() {
        let state = state(&["--initial-moisture", "42"]);
        request(
            &state,
            "POST",
            "/sim/state",
            Some(serde_json::json!({ "tank_percent": 30.0 })),
        )
        .await;
        let (_, read_back) = request(&state, "GET", "/sim/state", None).await;
        assert_eq!(read_back["moisture_vwc"], 42.0);
        assert_eq!(read_back["tank_percent"], 30.0);
    }

    #[tokio::test]
    async fn an_unknown_leak_state_is_refused_with_a_reason() {
        let state = state(&[]);
        let (status, body) = request(
            &state,
            "POST",
            "/sim/state",
            Some(serde_json::json!({ "leak": "damp" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("damp"));
    }

    #[tokio::test]
    async fn faults_can_be_enabled_and_disabled_at_runtime() {
        let state = state(&[]);
        let (status, body) = request(
            &state,
            "POST",
            "/sim/fault",
            Some(serde_json::json!({ "fault": "leak", "enabled": true })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["faults"].as_array().unwrap().contains(&"leak".into()));

        let (_, body) = request(
            &state,
            "POST",
            "/sim/fault",
            Some(serde_json::json!({ "fault": "leak", "enabled": false })),
        )
        .await;
        assert!(body["faults"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_parameterised_fault_is_accepted_in_the_same_spelling_as_the_flag() {
        let state = state(&[]);
        let (status, body) = request(
            &state,
            "POST",
            "/sim/fault",
            Some(serde_json::json!({ "fault": "clock-skew:-90" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body["faults"]
                .as_array()
                .unwrap()
                .contains(&"clock-skew:-90".into()),
            "`--fault` and `POST /sim/fault` accept exactly the same vocabulary"
        );
    }

    #[tokio::test]
    async fn an_unknown_fault_is_refused_rather_than_silently_ignored() {
        let state = state(&[]);
        let (status, body) = request(
            &state,
            "POST",
            "/sim/fault",
            Some(serde_json::json!({ "fault": "teleport" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("teleport"));
    }

    #[tokio::test]
    async fn restarting_advances_the_boot_count_and_clears_the_connection() {
        let state = state(&[]);
        let (_, before) = request(&state, "GET", "/sim/state", None).await;
        lock(&state.device).on_connected().unwrap();

        let signalled = Arc::clone(&state.restarted);
        let waiting = tokio::spawn(async move { signalled.notified().await });
        tokio::task::yield_now().await;

        let (status, after) = request(&state, "POST", "/sim/restart", None).await;
        assert_eq!(status, StatusCode::OK);
        tokio::time::timeout(std::time::Duration::from_secs(2), waiting)
            .await
            .expect("the run loop must be told to rebuild the connection")
            .unwrap();
        assert_eq!(
            after["boot_count"].as_u64().unwrap(),
            before["boot_count"].as_u64().unwrap() + 1
        );
        assert_eq!(after["connected"], false);
        assert_eq!(
            after["pump_running"], false,
            "a boot begins with the pump off"
        );
    }

    #[tokio::test]
    async fn the_scale_endpoint_reports_the_configured_factor() {
        let state = state(&["--time-scale", "600"]);
        let (status, body) = request(&state, "GET", "/sim/scale", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["time_scale"], 600.0);
    }

    /// There is no route that could start a dose, and there must never be one.
    #[tokio::test]
    async fn the_control_api_offers_no_way_to_actuate() {
        let state = state(&[]);
        for path in ["/sim/water", "/sim/pump", "/sim/dose", "/sim/actuate"] {
            let (status, _) = request(&state, "POST", path, Some(serde_json::json!({}))).await;
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "{path} must not exist: the control API is not a command path"
            );
        }
        // ...and setting environment state cannot move the pump either.
        request(
            &state,
            "POST",
            "/sim/state",
            Some(serde_json::json!({ "moisture_vwc": 0.0 })),
        )
        .await;
        let (_, body) = request(&state, "GET", "/sim/state", None).await;
        assert_eq!(body["pump_running"], false);
    }

    #[tokio::test]
    async fn a_persistent_state_fault_is_visible_through_the_api() {
        let settings = cli(&[]);
        std::fs::write(settings.resolved_state_file(), b"not a state file").unwrap();
        let state = ControlState::new(
            Arc::new(Mutex::new(Device::new(&settings))),
            Arc::new(settings),
        );
        let (_, body) = request(&state, "GET", "/sim/state", None).await;
        assert_eq!(body["actuation_permitted"], false);
        assert_eq!(body["persistent_state_fault"], "state_file_corrupt");
    }
}
