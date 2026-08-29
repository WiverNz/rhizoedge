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
const DEFAULT_EDGE_BIND: &str = "127.0.0.1:8080";
const DEFAULT_SIMULATOR_BIND: &str = "127.0.0.1:9090";

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
}

#[derive(Debug, Args)]
struct DeviceArgs {
    #[arg(default_value = "plant-node-01")]
    device_id: String,
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

impl Addresses {
    fn load(env_file: &Path) -> Result<Self> {
        let file_values = read_env_file(env_file)?;
        Ok(Self {
            edge: resolve_address(EDGE_BIND_KEY, DEFAULT_EDGE_BIND, &file_values)?,
            simulator: resolve_address(SIMULATOR_BIND_KEY, DEFAULT_SIMULATOR_BIND, &file_values)?,
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

async fn simulator_command(addresses: &Addresses, command: SimulatorCommand) -> Result<Value> {
    match command {
        SimulatorCommand::State => get(addresses.simulator, "/sim/state").await,
        SimulatorCommand::SetSoil(args) => {
            set_state(addresses, json!({ "moisture_vwc": args.moisture })).await
        }
        SimulatorCommand::Leak(args) => match args.state {
            Toggle::On => set_fault(addresses, "leak", true).await,
            Toggle::Off => {
                set_fault(addresses, "leak", false).await?;
                set_state(addresses, json!({ "leak": "clear" })).await
            }
        },
        SimulatorCommand::Tank(args) => match args.state {
            TankState::Empty => set_fault(addresses, "tank-empty", true).await,
            TankState::Restore => {
                set_fault(addresses, "tank-empty", false).await?;
                set_state(addresses, json!({ "tank_percent": args.percent })).await
            }
        },
        SimulatorCommand::Restart => post(addresses.simulator, "/sim/restart", None).await,
        SimulatorCommand::MissedWake(args) => {
            set_fault(addresses, format!("miss-wake:{}", args.count), true).await
        }
        SimulatorCommand::Disconnect(args) => {
            set_fault(addresses, format!("disconnect:{}", args.seconds), true).await
        }
        SimulatorCommand::Reconnect => set_fault(addresses, "disconnect:1", false).await,
    }
}

async fn edge_command(addresses: &Addresses, command: EdgeCommand) -> Result<Value> {
    match command {
        EdgeCommand::Readiness => get(addresses.edge, "/health/ready").await,
        EdgeCommand::DeviceState(args) => {
            if args.device_id.contains('/') {
                bail!("device id must not contain '/'");
            }
            get(
                addresses.edge,
                &format!("/api/v1/devices/{}", args.device_id),
            )
            .await
        }
    }
}

async fn scenario_command(addresses: &Addresses, command: ScenarioCommand) -> Result<Value> {
    match command {
        ScenarioCommand::DryPlant => set_state(addresses, json!({ "moisture_vwc": 20.0 })).await,
        ScenarioCommand::LeakWhileDry => {
            set_state(addresses, json!({ "moisture_vwc": 20.0 })).await?;
            set_fault(addresses, "leak", true).await
        }
        ScenarioCommand::BatteryMissedWake => set_fault(addresses, "miss-wake:1", true).await,
        ScenarioCommand::RecoverNormal => {
            for fault in ["leak", "tank-empty", "miss-wake:1", "disconnect:1"] {
                set_fault(addresses, fault, false).await?;
            }
            set_state(
                addresses,
                json!({ "moisture_vwc": 42.0, "tank_percent": 100.0, "leak": "clear" }),
            )
            .await
        }
    }
}

async fn event_command(addresses: &Addresses, event: Event) -> Result<Value> {
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

fn pretty(value: &Value) -> Result<String> {
    serde_json::to_string_pretty(value).context("could not format JSON response")
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let addresses = Addresses::load(&cli.env_file)?;
    let value = match cli.command {
        Command::Simulator { command } => simulator_command(&addresses, command).await?,
        Command::Edge { command } => edge_command(&addresses, command).await?,
        Command::Scenario { command } => scenario_command(&addresses, command).await?,
        Command::Event(args) => event_command(&addresses, args.event).await?,
    };
    println!("{}", pretty(&value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
