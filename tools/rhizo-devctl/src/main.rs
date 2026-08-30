//! Cross-platform development controls for a local Rhizo Edge topology.
//!
//! This wraps only the Edge REST API and the simulator's test-only control API.
//! It is not shipped with either production component and cannot actuate a pump.

#![cfg_attr(test, allow(clippy::unwrap_used))]

use std::collections::HashMap;
use std::env;
use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

const EDGE_BIND_KEY: &str = "RHIZO_EDGE__API__BIND";
const SIMULATOR_BIND_KEY: &str = "RHIZO_SIMULATOR__CONTROL_BIND";
const EDGE_STORAGE_KEY: &str = "RHIZO_EDGE__STORAGE__PATH";
const DEFAULT_EDGE_BIND: &str = "127.0.0.1:8080";
const DEFAULT_SIMULATOR_BIND: &str = "127.0.0.1:9090";
const DEFAULT_EDGE_STORAGE: &str = "./data/edge.sqlite";

#[derive(Debug, Parser)]
#[command(name = "rhizo-devctl", version, about)]
struct Cli {
    /// Repository environment file. Process environment values take precedence.
    #[arg(long, default_value = ".env", global = true)]
    env_file: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect or change the simulator through its test-only control API.
    Simulator {
        #[command(subcommand)]
        command: SimulatorCommand,
    },
    /// Inspect the running Edge Controller.
    Edge {
        #[command(subcommand)]
        command: EdgeCommand,
    },
    /// Apply a useful multi-step development scenario.
    Scenario {
        #[command(subcommand)]
        command: ScenarioCommand,
    },
    /// Apply one common event; intended for editor pick lists.
    Event(EventArgs),
    /// Delete disposable local Edge and simulator persistence.
    ResetLocalState(ResetLocalStateArgs),
}

#[derive(Debug, Args)]
struct ResetLocalStateArgs {
    /// Confirm deletion of the explicitly listed development-state files.
    #[arg(long)]
    confirm: bool,
}

#[derive(Debug, Args)]
struct EventArgs {
    #[arg(value_enum)]
    event: Event,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Event {
    State,
    DryPlant,
    LeakOn,
    LeakOff,
    TankEmpty,
    TankRestore,
    Restart,
    MissedWake,
    Disconnect,
    Reconnect,
    RecoverNormal,
}

#[derive(Debug, Subcommand)]
enum SimulatorCommand {
    /// Show the complete simulated environment and fault state.
    State,
    /// Set the true soil moisture percentage.
    SetSoil(SetSoilArgs),
    /// Enable or clear the simulated leak condition.
    Leak(ToggleArgs),
    /// Empty or restore the simulated reservoir.
    Tank(TankArgs),
    /// Reboot the simulated device without resetting its environment.
    Restart,
    /// Skip the next N battery wake cycles.
    MissedWake(MissedWakeArgs),
    /// Disconnect the simulated device for virtual seconds.
    Disconnect(DisconnectArgs),
    /// Clear an active disconnect fault so normal reconnection can resume.
    Reconnect,
}

#[derive(Debug, Args)]
struct SetSoilArgs {
    /// Volumetric water content percentage.
    #[arg(value_parser = parse_percentage)]
    moisture: f64,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Toggle {
    On,
    Off,
}

#[derive(Debug, Args)]
struct ToggleArgs {
    #[arg(value_enum)]
    state: Toggle,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TankState {
    Empty,
    Restore,
}

#[derive(Debug, Args)]
struct TankArgs {
    #[arg(value_enum)]
    state: TankState,
    /// Level used by `restore`.
    #[arg(long, default_value_t = 100.0, value_parser = parse_percentage)]
    percent: f64,
}

#[derive(Debug, Args)]
struct MissedWakeArgs {
    #[arg(default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    count: u32,
}

#[derive(Debug, Args)]
struct DisconnectArgs {
    /// Duration in virtual seconds.
    #[arg(default_value_t = 900, value_parser = clap::value_parser!(u32).range(1..))]
    seconds: u32,
}

#[derive(Debug, Subcommand)]
enum EdgeCommand {
    /// Check whether Edge has subscribed to MQTT and is ready.
    Readiness,
    /// Show one device from the Edge registry.
    DeviceState(DeviceArgs),
    /// Show the latest recommendation for one plant.
    PlantRecommendation(PlantArgs),
}

#[derive(Debug, Args)]
struct DeviceArgs {
    #[arg(default_value = "plant-node-01")]
    device_id: String,
}

#[derive(Debug, Args)]
struct PlantArgs {
    #[arg(default_value = "monstera-01")]
    plant_id: String,
}

#[derive(Debug, Subcommand)]
enum ScenarioCommand {
    /// Set a dry but otherwise normal plant environment.
    DryPlant,
    /// Set dry soil and a detected leak.
    LeakWhileDry,
    /// Make a battery node miss its next wake.
    BatteryMissedWake,
    /// Clear common faults and restore a normal environment.
    RecoverNormal,
}

#[derive(Debug)]
struct Addresses {
    edge: SocketAddr,
    simulator: SocketAddr,
}

#[derive(Debug)]
struct LocalConfig {
    values: HashMap<String, String>,
}

impl LocalConfig {
    fn load(env_file: &Path) -> Result<Self> {
        Ok(Self {
            values: read_env_file(env_file)?,
        })
    }

    fn value(&self, key: &str, default: &str) -> String {
        env::var(key)
            .ok()
            .or_else(|| self.values.get(key).cloned())
            .unwrap_or_else(|| default.to_owned())
    }

    fn addresses(&self) -> Result<Addresses> {
        Ok(Addresses {
            edge: resolve_address(EDGE_BIND_KEY, DEFAULT_EDGE_BIND, &self.values)?,
            simulator: resolve_address(SIMULATOR_BIND_KEY, DEFAULT_SIMULATOR_BIND, &self.values)?,
        })
    }
}

fn read_env_file(path: &Path) -> Result<HashMap<String, String>> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };
    let mut values = HashMap::new();
    for (index, raw) in contents.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            bail!("{}:{} is not KEY=VALUE", path.display(), index + 1);
        };
        values.insert(
            key.trim().to_owned(),
            value.trim().trim_matches(['\'', '"']).to_owned(),
        );
    }
    Ok(values)
}

fn resolve_address(
    key: &str,
    default: &str,
    file_values: &HashMap<String, String>,
) -> Result<SocketAddr> {
    let value = env::var(key)
        .ok()
        .or_else(|| file_values.get(key).cloned())
        .unwrap_or_else(|| default.to_owned());
    let address: SocketAddr = value
        .parse()
        .with_context(|| format!("{key} must be an IP socket address, got {value:?}"))?;
    Ok(client_address(address))
}

fn client_address(address: SocketAddr) -> SocketAddr {
    if address.ip().is_unspecified() {
        let loopback = match address.ip() {
            IpAddr::V4(_) => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            IpAddr::V6(_) => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        };
        SocketAddr::new(loopback, address.port())
    } else {
        address
    }
}

fn parse_percentage(value: &str) -> Result<f64, String> {
    let value: f64 = value.parse().map_err(|_| "expected a number".to_owned())?;
    if value.is_finite() && (0.0..=100.0).contains(&value) {
        Ok(value)
    } else {
        Err("expected a percentage from 0 through 100".to_owned())
    }
}

async fn request(
    address: SocketAddr,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let body = body.map_or_else(String::new, |value| value.to_string());
    let mut stream = TcpStream::connect(address).await.with_context(|| {
        format!("could not reach http://{address}{path}; is the service running?")
    })?;
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .context("could not send HTTP request")?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .context("could not read HTTP response")?;
    parse_response(&response)
}

fn parse_response(response: &[u8]) -> Result<Value> {
    let separator = b"\r\n\r\n";
    let split = response
        .windows(separator.len())
        .position(|window| window == separator)
        .context("HTTP response had no header terminator")?;
    let headers =
        std::str::from_utf8(&response[..split]).context("HTTP response headers were not UTF-8")?;
    let status = headers
        .lines()
        .next()
        .context("HTTP response had no status line")?;
    let code: u16 = status
        .split_whitespace()
        .nth(1)
        .context("HTTP response had no status code")?
        .parse()
        .context("HTTP response status was not numeric")?;
    let text = std::str::from_utf8(&response[split + separator.len()..])
        .context("HTTP response body was not UTF-8")?;
    let body = serde_json::from_str(text).unwrap_or_else(|_| json!({ "response": text }));
    if !(200..300).contains(&code) {
        bail!("request failed with HTTP {code}: {}", pretty(&body)?);
    }
    Ok(body)
}

async fn get(address: SocketAddr, path: &str) -> Result<Value> {
    request(address, "GET", path, None).await
}

async fn post(address: SocketAddr, path: &str, body: Option<Value>) -> Result<Value> {
    request(address, "POST", path, body).await
}

async fn set_state(addresses: &Addresses, body: Value) -> Result<Value> {
    post(addresses.simulator, "/sim/state", Some(body)).await
}

async fn set_fault(
    addresses: &Addresses,
    fault: impl Into<String>,
    enabled: bool,
) -> Result<Value> {
    post(
        addresses.simulator,
        "/sim/fault",
        Some(json!({ "fault": fault.into(), "enabled": enabled })),
    )
    .await
}

async fn simulator_state(addresses: &Addresses) -> Result<Value> {
    get(addresses.simulator, "/sim/state").await
}

async fn mutation_confirmation(addresses: &Addresses, label: &str) -> Result<String> {
    let state = simulator_state(addresses)
        .await
        .with_context(|| format!("{label} mutation succeeded, but state read-back failed"))?;
    format_confirmation(label, &state)
        .with_context(|| format!("{label} mutation succeeded, but state read-back was incomplete"))
}

fn format_confirmation(label: &str, state: &Value) -> Result<String> {
    let moisture = state_number(state, "moisture_vwc")?;
    let tank_percent = state_number(state, "tank_percent")?;
    let leak = match state_string(state, "leak")? {
        "detected" => "true",
        "clear" => "false",
        _ => "unknown",
    };
    let faults = state
        .get("faults")
        .and_then(Value::as_array)
        .context("simulator state field `faults` was not an array")?;
    let has_fault = |name: &str| {
        faults.iter().any(|fault| {
            fault
                .as_str()
                .is_some_and(|fault| fault == name || fault.starts_with(&format!("{name}:")))
        })
    };
    let tank_empty = has_fault("tank-empty") || tank_percent <= 0.0;

    let details = match label {
        "set-soil" | "dry-plant" => format!("moisture_vwc={moisture:.1}"),
        "leak-on" | "leak-off" => format!("leak={leak}"),
        "tank-empty" | "tank-restore" => {
            format!("tank_percent={tank_percent:.1} tank_empty={tank_empty}")
        }
        "restart" => format!(
            "boot_count={} connected={}",
            state_u64(state, "boot_count")?,
            state_bool(state, "connected")?
        ),
        "missed-wake" | "battery-missed-wake" => {
            format!(
                "missed_wake={} power_mode={}",
                has_fault("miss-wake"),
                state_string(state, "power_mode")?
            )
        }
        "disconnect" | "reconnect" => format!(
            "disconnect={} connected={}",
            has_fault("disconnect"),
            state_bool(state, "connected")?
        ),
        "leak-while-dry" => format!("moisture_vwc={moisture:.1} leak={leak}"),
        "recover-normal" => format!(
            "moisture_vwc={moisture:.1} leak={leak} tank_empty={tank_empty} tank_percent={tank_percent:.1} missed_wake={} disconnect={} connected={} faults={}",
            has_fault("miss-wake"),
            has_fault("disconnect"),
            state_bool(state, "connected")?,
            faults.len()
        ),
        other => bail!("no confirmation formatter for mutation {other:?}"),
    };
    Ok(format!("✓ {label} applied: {details}"))
}

fn state_number(state: &Value, field: &str) -> Result<f64> {
    state
        .get(field)
        .and_then(Value::as_f64)
        .with_context(|| format!("simulator state field `{field}` was not a number"))
}

fn state_string<'a>(state: &'a Value, field: &str) -> Result<&'a str> {
    state
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("simulator state field `{field}` was not a string"))
}

fn state_bool(state: &Value, field: &str) -> Result<bool> {
    state
        .get(field)
        .and_then(Value::as_bool)
        .with_context(|| format!("simulator state field `{field}` was not a boolean"))
}

fn state_u64(state: &Value, field: &str) -> Result<u64> {
    state
        .get(field)
        .and_then(Value::as_u64)
        .with_context(|| format!("simulator state field `{field}` was not an unsigned integer"))
}

enum Output {
    Json(Value),
    Confirmation(String),
}

async fn simulator_command(addresses: &Addresses, command: SimulatorCommand) -> Result<Output> {
    match command {
        SimulatorCommand::State => simulator_state(addresses).await.map(Output::Json),
        SimulatorCommand::SetSoil(args) => {
            set_state(addresses, json!({ "moisture_vwc": args.moisture }))
                .await
                .context("set-soil failed at mutation step")?;
            mutation_confirmation(addresses, "set-soil")
                .await
                .map(Output::Confirmation)
        }
        SimulatorCommand::Leak(args) => match args.state {
            Toggle::On => {
                set_fault(addresses, "leak", true)
                    .await
                    .context("leak-on failed at fault step")?;
                mutation_confirmation(addresses, "leak-on")
                    .await
                    .map(Output::Confirmation)
            }
            Toggle::Off => {
                set_fault(addresses, "leak", false)
                    .await
                    .context("leak-off failed at step 1/2: clear fault")?;
                set_state(addresses, json!({ "leak": "clear" }))
                    .await
                    .context("leak-off failed at step 2/2: clear physical sensor")?;
                mutation_confirmation(addresses, "leak-off")
                    .await
                    .map(Output::Confirmation)
            }
        },
        SimulatorCommand::Tank(args) => match args.state {
            TankState::Empty => {
                set_fault(addresses, "tank-empty", true)
                    .await
                    .context("tank-empty failed at fault step")?;
                mutation_confirmation(addresses, "tank-empty")
                    .await
                    .map(Output::Confirmation)
            }
            TankState::Restore => {
                set_fault(addresses, "tank-empty", false)
                    .await
                    .context("tank-restore failed at step 1/2: clear fault")?;
                set_state(addresses, json!({ "tank_percent": args.percent }))
                    .await
                    .context("tank-restore failed at step 2/2: refill tank")?;
                mutation_confirmation(addresses, "tank-restore")
                    .await
                    .map(Output::Confirmation)
            }
        },
        SimulatorCommand::Restart => {
            post(addresses.simulator, "/sim/restart", None)
                .await
                .context("restart failed at mutation step")?;
            mutation_confirmation(addresses, "restart")
                .await
                .map(Output::Confirmation)
        }
        SimulatorCommand::MissedWake(args) => {
            set_fault(addresses, format!("miss-wake:{}", args.count), true)
                .await
                .context("missed-wake failed at fault step")?;
            mutation_confirmation(addresses, "missed-wake")
                .await
                .map(Output::Confirmation)
        }
        SimulatorCommand::Disconnect(args) => {
            set_fault(addresses, format!("disconnect:{}", args.seconds), true)
                .await
                .context("disconnect failed at fault step")?;
            mutation_confirmation(addresses, "disconnect")
                .await
                .map(Output::Confirmation)
        }
        SimulatorCommand::Reconnect => {
            set_fault(addresses, "disconnect:1", false)
                .await
                .context("reconnect failed at fault step")?;
            mutation_confirmation(addresses, "reconnect")
                .await
                .map(Output::Confirmation)
        }
    }
}

async fn edge_command(addresses: &Addresses, command: EdgeCommand) -> Result<Output> {
    match command {
        EdgeCommand::Readiness => get(addresses.edge, "/health/ready").await.map(Output::Json),
        EdgeCommand::DeviceState(args) => {
            validate_path_id("device", &args.device_id)?;
            get(
                addresses.edge,
                &format!("/api/v1/devices/{}", args.device_id),
            )
            .await
            .map(Output::Json)
        }
        EdgeCommand::PlantRecommendation(args) => {
            validate_path_id("plant", &args.plant_id)?;
            get(
                addresses.edge,
                &format!("/api/v1/plants/{}/recommendation", args.plant_id),
            )
            .await
            .map(Output::Json)
        }
    }
}

fn validate_path_id(kind: &str, value: &str) -> Result<()> {
    if value.contains('/') {
        bail!("{kind} id must not contain '/'");
    }
    Ok(())
}

async fn scenario_command(addresses: &Addresses, command: ScenarioCommand) -> Result<Output> {
    match command {
        ScenarioCommand::DryPlant => {
            set_state(addresses, json!({ "moisture_vwc": 20.0 }))
                .await
                .context("dry-plant failed at step 1/1: set moisture")?;
            mutation_confirmation(addresses, "dry-plant")
                .await
                .map(Output::Confirmation)
        }
        ScenarioCommand::LeakWhileDry => {
            set_state(addresses, json!({ "moisture_vwc": 20.0 }))
                .await
                .context("leak-while-dry failed at step 1/2: set moisture")?;
            set_fault(addresses, "leak", true)
                .await
                .context("leak-while-dry failed at step 2/2: enable leak")?;
            mutation_confirmation(addresses, "leak-while-dry")
                .await
                .map(Output::Confirmation)
        }
        ScenarioCommand::BatteryMissedWake => {
            set_fault(addresses, "miss-wake:1", true)
                .await
                .context("battery-missed-wake failed at step 1/1: enable missed wake")?;
            mutation_confirmation(addresses, "battery-missed-wake")
                .await
                .map(Output::Confirmation)
        }
        ScenarioCommand::RecoverNormal => {
            for fault in ["leak", "tank-empty", "miss-wake:1", "disconnect:1"] {
                set_fault(addresses, fault, false)
                    .await
                    .with_context(|| format!("recover-normal failed while clearing {fault}"))?;
            }
            set_state(
                addresses,
                json!({ "moisture_vwc": 42.0, "tank_percent": 100.0, "leak": "clear" }),
            )
            .await
            .context("recover-normal failed while restoring physical state")?;
            mutation_confirmation(addresses, "recover-normal")
                .await
                .map(Output::Confirmation)
        }
    }
}

async fn event_command(addresses: &Addresses, event: Event) -> Result<Output> {
    match event {
        Event::State => simulator_command(addresses, SimulatorCommand::State).await,
        Event::DryPlant => scenario_command(addresses, ScenarioCommand::DryPlant).await,
        Event::LeakOn => {
            simulator_command(
                addresses,
                SimulatorCommand::Leak(ToggleArgs { state: Toggle::On }),
            )
            .await
        }
        Event::LeakOff => {
            simulator_command(
                addresses,
                SimulatorCommand::Leak(ToggleArgs { state: Toggle::Off }),
            )
            .await
        }
        Event::TankEmpty => {
            simulator_command(
                addresses,
                SimulatorCommand::Tank(TankArgs {
                    state: TankState::Empty,
                    percent: 100.0,
                }),
            )
            .await
        }
        Event::TankRestore => {
            simulator_command(
                addresses,
                SimulatorCommand::Tank(TankArgs {
                    state: TankState::Restore,
                    percent: 100.0,
                }),
            )
            .await
        }
        Event::Restart => simulator_command(addresses, SimulatorCommand::Restart).await,
        Event::MissedWake => {
            simulator_command(
                addresses,
                SimulatorCommand::MissedWake(MissedWakeArgs { count: 1 }),
            )
            .await
        }
        Event::Disconnect => {
            simulator_command(
                addresses,
                SimulatorCommand::Disconnect(DisconnectArgs { seconds: 900 }),
            )
            .await
        }
        Event::Reconnect => simulator_command(addresses, SimulatorCommand::Reconnect).await,
        Event::RecoverNormal => scenario_command(addresses, ScenarioCommand::RecoverNormal).await,
    }
}

async fn reset_local_state(config: &LocalConfig, confirmed: bool) -> Result<Value> {
    if !confirmed {
        bail!(
            "refusing to delete local state without --confirm; stop Edge and all simulators first"
        );
    }
    let addresses = config.addresses()?;
    for (name, address) in [("Edge", addresses.edge), ("simulator", addresses.simulator)] {
        if tokio::time::timeout(
            std::time::Duration::from_millis(250),
            TcpStream::connect(address),
        )
        .await
        .is_ok_and(|result| result.is_ok())
        {
            bail!(
                "{name} is still listening at {address}; stop the VS Code debug session before resetting state"
            );
        }
    }

    let root = env::current_dir().context("could not resolve the workspace directory")?;
    let storage = PathBuf::from(config.value(EDGE_STORAGE_KEY, DEFAULT_EDGE_STORAGE));
    let storage = checked_local_path(&root, &storage)?;
    let mut targets = vec![
        storage.clone(),
        PathBuf::from(format!("{}-wal", storage.display())),
        PathBuf::from(format!("{}-shm", storage.display())),
        PathBuf::from(format!("{}.pre-migration.bak", storage.display())),
    ];

    let device_ids = config.value("DEVICE_IDS", "plant-node-01,plant-node-02");
    for device_id in device_ids
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        if !device_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        {
            bail!("DEVICE_IDS contains unsafe filename component {device_id:?}");
        }
        for suffix in [".state.json", ".state.json.tmp", ".state.json.corrupt"] {
            targets.push(checked_local_path(
                &root,
                &PathBuf::from(format!("{device_id}{suffix}")),
            )?);
        }
    }

    let mut removed = Vec::new();
    for target in targets {
        match fs::remove_file(&target) {
            Ok(()) => removed.push(relative_display(&root, &target)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("could not remove {}", target.display()));
            }
        }
    }
    Ok(json!({
        "status": "local development state reset",
        "removed": removed,
        "next": "restart Edge and the simulator; migrations will recreate the database"
    }))
}

fn checked_local_path(root: &Path, configured: &Path) -> Result<PathBuf> {
    if configured
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!(
            "refusing state path containing '..': {}",
            configured.display()
        );
    }
    let root = root
        .canonicalize()
        .with_context(|| format!("could not resolve workspace {}", root.display()))?;
    let target = if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        root.join(configured)
    };
    if configured.is_absolute() && !target.starts_with(&root) {
        bail!(
            "refusing to remove state outside workspace {}: {}",
            root.display(),
            target.display()
        );
    }
    let target_parent = target
        .parent()
        .context("state path has no parent directory")?;
    let mut existing_ancestor = target_parent;
    while !existing_ancestor.exists() {
        existing_ancestor = existing_ancestor.parent().with_context(|| {
            format!(
                "could not find an existing parent directory for {}",
                target.display()
            )
        })?;
    }
    let resolved_ancestor = existing_ancestor
        .canonicalize()
        .with_context(|| format!("could not resolve state directory for {}", target.display()))?;
    if !resolved_ancestor.starts_with(&root) {
        bail!(
            "refusing to remove state outside workspace {}: {}",
            root.display(),
            target.display()
        );
    }
    Ok(target)
}

fn relative_display(root: &Path, target: &Path) -> String {
    target
        .strip_prefix(root)
        .unwrap_or(target)
        .display()
        .to_string()
}

fn pretty(value: &Value) -> Result<String> {
    serde_json::to_string_pretty(value).context("could not format JSON response")
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = LocalConfig::load(&cli.env_file)?;
    let addresses = config.addresses()?;
    let output = match cli.command {
        Command::Simulator { command } => simulator_command(&addresses, command).await?,
        Command::Edge { command } => edge_command(&addresses, command).await?,
        Command::Scenario { command } => scenario_command(&addresses, command).await?,
        Command::Event(args) => event_command(&addresses, args.event).await?,
        Command::ResetLocalState(args) => {
            Output::Json(reset_local_state(&config, args.confirm).await?)
        }
    };
    match output {
        Output::Json(value) => println!("{}", pretty(&value)?),
        Output::Confirmation(message) => println!("{message}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const READ_BACK_STATE: &str = r#"{
        "device_id":"plant-node-01",
        "moisture_vwc":19.5,
        "tank_percent":100.0,
        "leak":"clear",
        "faults":[],
        "boot_count":2,
        "connected":true,
        "power_mode":"always_on"
    }"#;

    async fn mock_http_server(
        responses: Vec<(u16, &'static str)>,
    ) -> (SocketAddr, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let read = stream.read(&mut chunk).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..read]);
                    if complete_http_request(&request) {
                        break;
                    }
                }
                requests.push(String::from_utf8(request).unwrap());
                let reason = if status == 200 {
                    "OK"
                } else {
                    "Internal Server Error"
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        (address, task)
    }

    fn complete_http_request(request: &[u8]) -> bool {
        let Some(split) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let headers = String::from_utf8_lossy(&request[..split]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length: ")
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(0);
        request.len() >= split + 4 + content_length
    }

    #[test]
    fn unspecified_bind_addresses_become_connectable_loopback_addresses() {
        assert_eq!(
            client_address("0.0.0.0:8123".parse().unwrap()),
            "127.0.0.1:8123".parse().unwrap()
        );
        assert_eq!(
            client_address("[::]:8123".parse().unwrap()),
            "[::1]:8123".parse().unwrap()
        );
    }

    #[test]
    fn percentages_are_bounded() {
        assert_eq!(parse_percentage("20").unwrap(), 20.0);
        assert!(parse_percentage("-1").is_err());
        assert!(parse_percentage("101").is_err());
        assert!(parse_percentage("NaN").is_err());
    }

    #[test]
    fn parses_a_successful_json_response() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{\"status\":\"ok\"}";
        assert_eq!(parse_response(response).unwrap()["status"], "ok");
    }

    #[tokio::test]
    async fn dry_plant_confirmation_uses_a_fresh_state_read_back() {
        let (address, server) = mock_http_server(vec![(200, "{}"), (200, READ_BACK_STATE)]).await;
        let addresses = Addresses {
            edge: address,
            simulator: address,
        };

        let output = scenario_command(&addresses, ScenarioCommand::DryPlant)
            .await
            .unwrap();
        let Output::Confirmation(message) = output else {
            panic!("a mutation must produce a concise confirmation");
        };
        assert_eq!(message, "✓ dry-plant applied: moisture_vwc=19.5");

        let requests = server.await.unwrap();
        assert!(requests[0].starts_with("POST /sim/state "));
        assert!(requests[1].starts_with("GET /sim/state "));
    }

    #[tokio::test]
    async fn a_partial_scenario_failure_names_the_failed_step_and_skips_read_back() {
        let (address, server) = mock_http_server(vec![
            (200, "{}"),
            (500, r#"{"error":"fault injection failed"}"#),
        ])
        .await;
        let addresses = Addresses {
            edge: address,
            simulator: address,
        };

        let error = scenario_command(&addresses, ScenarioCommand::LeakWhileDry)
            .await
            .err()
            .unwrap();
        assert!(
            error
                .to_string()
                .contains("leak-while-dry failed at step 2/2: enable leak"),
            "{error:#}"
        );

        let requests = server.await.unwrap();
        assert_eq!(
            requests.len(),
            2,
            "failed scenarios must not claim read-back"
        );
        assert!(requests[0].starts_with("POST /sim/state "));
        assert!(requests[1].starts_with("POST /sim/fault "));
    }

    #[tokio::test]
    async fn plant_recommendation_uses_the_existing_edge_endpoint() {
        let (address, server) =
            mock_http_server(vec![(200, r#"{"recommendation":"water"}"#)]).await;
        let addresses = Addresses {
            edge: address,
            simulator: address,
        };

        let output = edge_command(
            &addresses,
            EdgeCommand::PlantRecommendation(PlantArgs {
                plant_id: "monstera-01".to_owned(),
            }),
        )
        .await
        .unwrap();
        let Output::Json(value) = output else {
            panic!("an Edge query must retain its structured JSON response");
        };
        assert_eq!(value["recommendation"], "water");

        let requests = server.await.unwrap();
        assert!(requests[0].starts_with("GET /api/v1/plants/monstera-01/recommendation "));
    }

    #[tokio::test]
    async fn reset_requires_explicit_confirmation_before_any_deletion() {
        let config = LocalConfig {
            values: HashMap::new(),
        };
        let error = reset_local_state(&config, false).await.unwrap_err();
        assert!(error.to_string().contains("without --confirm"));
    }

    #[test]
    fn reset_paths_cannot_escape_the_workspace() {
        let root = env::current_dir().unwrap();
        assert!(checked_local_path(&root, Path::new("../edge.sqlite")).is_err());
        assert!(checked_local_path(&root, Path::new("data/edge.sqlite")).is_ok());
    }
}
