//! Command-line interface for the reference device simulator.
//!
//! Every flag documented in
//! [simulator-strategy.md](../../../../docs/testing/simulator-strategy.md) §6
//! and §7 is defined here, including the ones whose behaviour lands in a later
//! issue. Defining the whole surface at once keeps it stable while the
//! implementation fills in, rather than churning the interface across five
//! issues.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

use clap::Parser;
use rhizo_mqtt_contract::DeviceId;
use rhizo_mqtt_contract::payload::{ActuatorKind, SensorId};

/// The default control-API port (PRD 020 §Interfaces).
pub const DEFAULT_CONTROL_PORT: u16 = 9090;

/// A simulated sensor group.
///
/// A group, not a measurement kind: real hardware is a probe that produces
/// several kinds at once, and `--sensors` selects hardware. The kinds each
/// group produces are declared once in `capabilities`, which is also what
/// drives sampling — a device must not declare what it does not publish.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SensorGroup {
    /// Soil probe: moisture, temperature, EC.
    Soil,
    /// Pot scale: weight.
    Weight,
    /// Reservoir level.
    Tank,
    /// Tray leak detector.
    Leak,
    /// The device's own supply gauge. Declared automatically by
    /// `--power-mode battery`; selectable here for a mains device that still
    /// reports a backup pack.
    Battery,
}

impl SensorGroup {
    /// Every group, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::Soil,
        Self::Weight,
        Self::Tank,
        Self::Leak,
        Self::Battery,
    ];

    /// The lowercase name accepted by `--sensors`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Soil => "soil",
            Self::Weight => "weight",
            Self::Tank => "tank",
            Self::Leak => "leak",
            Self::Battery => "battery",
        }
    }
}

impl fmt::Display for SensorGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SensorGroup {
    type Err = CliError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|g| g.as_str() == s)
            .ok_or_else(|| CliError::UnknownSensor(s.to_owned()))
    }
}

/// The value of `--sensors`: a comma-separated list, possibly empty.
///
/// A newtype rather than a `Vec` field with a `value_delimiter` because the
/// empty list has to be expressible. `--sensors ''` is a device with no
/// sensors, which is a configuration tests need, and a delimited `Vec` parses
/// that as one empty-named group.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SensorList(pub Vec<SensorGroup>);

impl SensorList {
    /// The selected groups.
    #[must_use]
    pub fn groups(&self) -> &[SensorGroup] {
        &self.0
    }

    /// Whether a group is enabled.
    #[must_use]
    pub fn contains(&self, group: SensorGroup) -> bool {
        self.0.contains(&group)
    }
}

impl fmt::Display for SensorList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&render_list(&self.0))
    }
}

impl FromStr for SensorList {
    type Err = CliError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        split_list(s).map(SensorList)
    }
}

/// The value of `--actuators`: a comma-separated list, possibly empty.
///
/// An empty list is a **monitoring-only device**, which is the shape most real
/// plants have rather than an edge case to tolerate (SAFETY-018).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActuatorList(pub Vec<ActuatorSpec>);

impl ActuatorList {
    /// The declared actuators.
    #[must_use]
    pub fn specs(&self) -> &[ActuatorSpec] {
        &self.0
    }
}

impl fmt::Display for ActuatorList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&render_list(
            &self
                .0
                .iter()
                .map(|a| a.actuator_id.as_str())
                .collect::<Vec<_>>(),
        ))
    }
}

impl FromStr for ActuatorList {
    type Err = CliError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        split_list(s).map(ActuatorList)
    }
}

/// Splits a comma list, treating an empty or whitespace-only value as none.
fn split_list<T: FromStr<Err = CliError>>(s: &str) -> Result<Vec<T>, CliError> {
    s.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(T::from_str)
        .collect()
}

/// Renders a list for help text and structured log fields.
fn render_list<T: fmt::Display>(items: &[T]) -> String {
    if items.is_empty() {
        return String::from("(none)");
    }
    items
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// A declared actuator, parsed from `--actuators`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActuatorSpec {
    /// Stable actuator id, declared in status and referenced by policies.
    pub actuator_id: SensorId,
    /// Actuator kind. `irrigation_pump` is the only kind V1 implements.
    pub kind: ActuatorKind,
}

impl FromStr for ActuatorSpec {
    type Err = CliError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (id, kind) = s.split_once(':').unwrap_or((s, "irrigation_pump"));
        let actuator_id =
            SensorId::parse(id).map_err(|_| CliError::InvalidActuatorId(id.to_owned()))?;
        let kind = match kind {
            "irrigation_pump" => ActuatorKind::IrrigationPump,
            "valve" => ActuatorKind::Valve,
            "grow_light" => ActuatorKind::GrowLight,
            "fan" => ActuatorKind::Fan,
            "heater" => ActuatorKind::Heater,
            "humidifier" => ActuatorKind::Humidifier,
            "fertiliser_dosing_pump" => ActuatorKind::FertiliserDosingPump,
            other => return Err(CliError::UnknownActuatorKind(other.to_owned())),
        };
        Ok(Self { actuator_id, kind })
    }
}

/// A step of the policy `validate -> stage -> verify -> activate -> acknowledge`
/// sequence, addressable by `--fault policy-interrupt:<step>` (M2-016).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PolicyStep {
    /// After validation, before anything is written.
    Validate,
    /// After the staging region is written.
    Stage,
    /// After the staging read-back verifies.
    Verify,
    /// After the active pointer flips.
    Activate,
    /// After the applied version is acknowledged in status.
    Acknowledge,
}

impl PolicyStep {
    /// Every step, in execution order.
    pub const ALL: [Self; 5] = [
        Self::Validate,
        Self::Stage,
        Self::Verify,
        Self::Activate,
        Self::Acknowledge,
    ];

    /// The lowercase name accepted by `--fault policy-interrupt:<step>`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Stage => "stage",
            Self::Verify => "verify",
            Self::Activate => "activate",
            Self::Acknowledge => "acknowledge",
        }
    }
}

impl fmt::Display for PolicyStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PolicyStep {
    type Err = CliError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|step| step.as_str() == s)
            .ok_or_else(|| CliError::UnknownPolicyStep(s.to_owned()))
    }
}

/// One injectable fault.
///
/// The catalogue is simulator-strategy.md §6 plus `policy-interrupt`, added by
/// M2-016 for the activation sequence. Parsing lives here so `--fault` and
/// `POST /sim/fault` accept exactly the same vocabulary — two spellings of the
/// same fault would be two things to keep in step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fault {
    /// Drop the MQTT connection for N seconds of virtual time.
    Disconnect {
        /// Isolation duration.
        seconds: u32,
    },
    /// Publish a fraction of messages twice with an identical `message_id`.
    Duplicate {
        /// Probability in `0.0..=1.0`.
        rate: f64,
    },
    /// Delay a fraction of messages past the next one.
    Reorder {
        /// Probability in `0.0..=1.0`.
        rate: f64,
    },
    /// Emit out-of-range or null moisture at the given rate.
    InvalidSoil {
        /// Probability in `0.0..=1.0`.
        rate: f64,
    },
    /// Omit soil moisture while continuing every other telemetry stream.
    StaleSoil,
    /// Omit tank level while continuing every other telemetry stream.
    StaleTank,
    /// Omit leak state while continuing every other telemetry stream.
    StaleLeak,
    /// Omit pot weight while continuing every other telemetry stream.
    StaleWeight,
    /// Repeat one bit-identical reading forever.
    StuckSensor,
    /// Report `clock_synced: false` regardless of synchronisation.
    ClockUnsync,
    /// Offset `device_time_ms` by N seconds.
    ClockSkew {
        /// Signed offset.
        seconds: i64,
    },
    /// Assert the leak sensor.
    Leak,
    /// Drive the tank level to zero.
    TankEmpty,
    /// Run the pump without delivering water.
    PumpNoDelivery,
    /// Fail to de-energise the pump, exercising the independent run guard.
    PumpStuckOn,
    /// Terminate the process during actuation, after the state write.
    RestartMidDose,
    /// Drop the broker socket after accepting a dose, before its result.
    DisconnectMidDose,
    /// Full restart with a fresh `boot_id`.
    Restart,
    /// Terminate during policy activation at the named step.
    PolicyInterrupt {
        /// Step after which the process dies.
        step: PolicyStep,
    },
    /// Sleep through the next `count` wake cycles without announcing anything,
    /// so an edge sees a device that stopped waking (SCEN-111).
    MissWake {
        /// Consecutive wake cycles to skip.
        count: u32,
    },
    /// Leave the broker from battery mode without publishing the sleep
    /// announcement, so the Last Will fires and the absence is unexplained
    /// (SCEN-112).
    SleepWithoutAnnouncing,
}

impl Fault {
    /// The stable name used by `--fault` and `POST /sim/fault`, without any
    /// parameter.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Disconnect { .. } => "disconnect",
            Self::Duplicate { .. } => "duplicate",
            Self::Reorder { .. } => "reorder",
            Self::InvalidSoil { .. } => "invalid-soil",
            Self::StaleSoil => "stale-soil",
            Self::StaleTank => "stale-tank",
            Self::StaleLeak => "stale-leak",
            Self::StaleWeight => "stale-weight",
            Self::StuckSensor => "stuck-sensor",
            Self::ClockUnsync => "clock-unsync",
            Self::ClockSkew { .. } => "clock-skew",
            Self::Leak => "leak",
            Self::TankEmpty => "tank-empty",
            Self::PumpNoDelivery => "pump-no-delivery",
            Self::PumpStuckOn => "pump-stuck-on",
            Self::RestartMidDose => "restart-mid-dose",
            Self::DisconnectMidDose => "disconnect-mid-dose",
            Self::Restart => "restart",
            Self::PolicyInterrupt { .. } => "policy-interrupt",
            Self::MissWake { .. } => "miss-wake",
            Self::SleepWithoutAnnouncing => "sleep-without-announcing",
        }
    }

    /// Every fault specification in the catalogue, for help text and tests.
    pub const NAMES: [&'static str; 21] = [
        "disconnect:<sec>",
        "duplicate:<rate>",
        "reorder:<rate>",
        "invalid-soil:<rate>",
        "stale-soil",
        "stale-tank",
        "stale-leak",
        "stale-weight",
        "stuck-sensor",
        "clock-unsync",
        "clock-skew:<sec>",
        "leak",
        "tank-empty",
        "pump-no-delivery",
        "pump-stuck-on",
        "restart-mid-dose",
        "disconnect-mid-dose",
        "restart",
        "policy-interrupt:<step>",
        "miss-wake:<n>",
        "sleep-without-announcing",
    ];
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnect { seconds } => write!(f, "disconnect:{seconds}"),
            Self::Duplicate { rate } => write!(f, "duplicate:{rate}"),
            Self::Reorder { rate } => write!(f, "reorder:{rate}"),
            Self::InvalidSoil { rate } => write!(f, "invalid-soil:{rate}"),
            Self::StaleSoil => f.write_str("stale-soil"),
            Self::StaleTank => f.write_str("stale-tank"),
            Self::StaleLeak => f.write_str("stale-leak"),
            Self::StaleWeight => f.write_str("stale-weight"),
            Self::ClockSkew { seconds } => write!(f, "clock-skew:{seconds}"),
            Self::PolicyInterrupt { step } => write!(f, "policy-interrupt:{step}"),
            Self::MissWake { count } => write!(f, "miss-wake:{count}"),
            other => f.write_str(other.name()),
        }
    }
}

impl FromStr for Fault {
    type Err = CliError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, arg) = match s.split_once(':') {
            Some((name, arg)) => (name, Some(arg)),
            None => (s, None),
        };
        // Free functions rather than closures: each needs the `'a` of `arg`
        // to outlive the call, which a closure over `name` cannot express.
        fn need<'a>(name: &str, arg: Option<&'a str>) -> Result<&'a str, CliError> {
            arg.ok_or_else(|| CliError::FaultNeedsArgument(name.to_owned()))
        }
        fn number<T: FromStr>(spec: &str, name: &str, arg: Option<&str>) -> Result<T, CliError> {
            need(name, arg)?
                .parse()
                .map_err(|_| CliError::FaultArgument(spec.to_owned()))
        }
        fn rate(spec: &str, name: &str, arg: Option<&str>) -> Result<f64, CliError> {
            let value: f64 = number(spec, name, arg)?;
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(CliError::FaultRateRange(spec.to_owned()));
            }
            Ok(value)
        }
        match name {
            "disconnect" => Ok(Self::Disconnect {
                seconds: number(s, name, arg)?,
            }),
            "duplicate" => Ok(Self::Duplicate {
                rate: rate(s, name, arg)?,
            }),
            "reorder" => Ok(Self::Reorder {
                rate: rate(s, name, arg)?,
            }),
            "invalid-soil" => Ok(Self::InvalidSoil {
                rate: rate(s, name, arg)?,
            }),
            "stale-soil" => Ok(Self::StaleSoil),
            "stale-tank" => Ok(Self::StaleTank),
            "stale-leak" => Ok(Self::StaleLeak),
            "stale-weight" => Ok(Self::StaleWeight),
            "stuck-sensor" => Ok(Self::StuckSensor),
            "clock-unsync" => Ok(Self::ClockUnsync),
            "clock-skew" => Ok(Self::ClockSkew {
                seconds: number(s, name, arg)?,
            }),
            "leak" => Ok(Self::Leak),
            "tank-empty" => Ok(Self::TankEmpty),
            "pump-no-delivery" => Ok(Self::PumpNoDelivery),
            "pump-stuck-on" => Ok(Self::PumpStuckOn),
            "restart-mid-dose" => Ok(Self::RestartMidDose),
            "disconnect-mid-dose" => Ok(Self::DisconnectMidDose),
            "restart" => Ok(Self::Restart),
            "policy-interrupt" => Ok(Self::PolicyInterrupt {
                step: need(name, arg)?.parse()?,
            }),
            "miss-wake" => Ok(Self::MissWake {
                count: number(s, name, arg)?,
            }),
            "sleep-without-announcing" => Ok(Self::SleepWithoutAnnouncing),
            other => Err(CliError::UnknownFault(other.to_owned())),
        }
    }
}

/// The `--power-mode` value.
///
/// A separate type from the contract's `PowerMode` because the command line
/// admits only the two modes a simulator can actually run; `unknown` is a wire
/// value, not something anyone can ask for.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PowerModeArg {
    /// Stay connected, as every device did before ADR-018.
    #[default]
    AlwaysOn,
    /// Sleep between sampling cycles.
    Battery,
}

impl PowerModeArg {
    /// The wire value this mode declares.
    #[must_use]
    pub const fn wire(self) -> rhizo_mqtt_contract::payload::PowerMode {
        match self {
            Self::AlwaysOn => rhizo_mqtt_contract::payload::PowerMode::AlwaysOn,
            Self::Battery => rhizo_mqtt_contract::payload::PowerMode::Battery,
        }
    }
    /// Whether this device sleeps.
    #[must_use]
    pub const fn is_battery(self) -> bool {
        matches!(self, Self::Battery)
    }
}

impl fmt::Display for PowerModeArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::AlwaysOn => "always_on",
            Self::Battery => "battery",
        })
    }
}

impl FromStr for PowerModeArg {
    type Err = CliError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "always_on" | "always-on" => Ok(Self::AlwaysOn),
            "battery" => Ok(Self::Battery),
            other => Err(CliError::UnknownPowerMode(other.to_owned())),
        }
    }
}

/// Why an argument was rejected.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CliError {
    /// The simulator-only HTTP control surface was configured beyond loopback.
    #[error("--control-bind must use a loopback IP address, got {0}")]
    ControlBindMustBeLoopback(SocketAddr),
    /// `--sensors` named a group that does not exist.
    #[error("unknown sensor group `{0}`; expected one of soil, weight, tank, leak, battery")]
    UnknownSensor(String),
    /// `--actuators` named an id that is not a valid local identifier.
    #[error("invalid actuator id `{0}`")]
    InvalidActuatorId(String),
    /// `--actuators` named a kind outside the protocol enumeration.
    #[error("unknown actuator kind `{0}`")]
    UnknownActuatorKind(String),
    /// `--power-mode` named a mode that does not exist.
    #[error("unknown power mode `{0}`; expected always_on or battery")]
    UnknownPowerMode(String),
    /// `--fault` named a fault that is not in the catalogue.
    #[error("unknown fault `{0}`")]
    UnknownFault(String),
    /// A parameterised fault was given without its parameter.
    #[error("fault `{0}` requires an argument, e.g. `{0}:1`")]
    FaultNeedsArgument(String),
    /// A fault parameter did not parse.
    #[error("could not parse the argument of fault `{0}`")]
    FaultArgument(String),
    /// A rate parameter was outside `0.0..=1.0`.
    #[error("fault `{0}` takes a rate between 0.0 and 1.0")]
    FaultRateRange(String),
    /// `policy-interrupt` named a step that does not exist.
    #[error("unknown policy step `{0}`; expected validate, stage, verify, activate, acknowledge")]
    UnknownPolicyStep(String),
    /// `--time-scale` was zero, negative, or not finite.
    #[error("--time-scale must be finite and greater than zero")]
    TimeScale,
    /// A physical-model parameter was outside its plausible range.
    #[error("--{flag} must be {expected}")]
    ModelParameter {
        /// The offending flag, without its leading dashes.
        flag: &'static str,
        /// What would have been accepted.
        expected: &'static str,
    },
}

/// A protocol-identical simulated plant node.
///
/// Runs against a bare broker: the edge controller is not a dependency.
#[derive(Clone, Debug, Parser)]
#[command(name = "device-simulator", version, about, long_about = None)]
pub struct Cli {
    /// Device identity. Also the MQTT client id and broker username (ADR-012).
    #[arg(long, value_name = "ID")]
    pub device_id: DeviceId,

    /// Broker URL, `mqtt://host:port`.
    #[arg(long, value_name = "URL", default_value = "mqtt://localhost:1883")]
    pub broker: String,

    /// Broker username. Defaults to `--device-id`, which is what the ACL
    /// pattern `rhizo/v1/devices/%u/#` confines.
    #[arg(long, value_name = "NAME")]
    pub username: Option<String>,

    /// Broker password.
    #[arg(long, value_name = "SECRET", env = "RHIZO_DEVICE_PASSWORD")]
    pub password: Option<String>,

    /// Initial soil moisture, volumetric water content percent.
    #[arg(long, value_name = "VWC", default_value_t = 42.0)]
    pub initial_moisture: f64,

    /// Drying rate constant, per hour.
    #[arg(long, value_name = "PER_HOUR", default_value_t = 0.06)]
    pub drying_rate: f64,

    /// Pot volume in millilitres.
    #[arg(long, value_name = "ML", default_value_t = 2500.0)]
    pub pot_volume_ml: f64,

    /// Tank capacity in millilitres.
    #[arg(long, value_name = "ML", default_value_t = 2000.0)]
    pub tank_capacity_ml: f64,

    /// Pump calibration in millilitres per second.
    #[arg(long, value_name = "ML_PER_S", default_value_t = 8.2)]
    pub ml_per_second: f32,

    /// Sampling interval in seconds of virtual time.
    #[arg(long, value_name = "SECONDS", default_value_t = 300)]
    pub telemetry_interval: u32,

    /// Virtual-time acceleration factor. 600 runs ten simulated minutes per
    /// real second.
    #[arg(long, value_name = "FACTOR", default_value_t = 1.0)]
    pub time_scale: f64,

    /// Simulated sensor groups, comma separated. Pass an empty value for a
    /// device with no sensors.
    #[arg(long, value_name = "LIST", default_value = "soil,weight,tank,leak")]
    pub sensors: SensorList,

    /// Declared actuators, comma separated as `id` or `id:kind`. Pass an empty
    /// value for a monitoring-only device, which is a normal device.
    #[arg(long, value_name = "LIST", default_value = "pump-0")]
    pub actuators: ActuatorList,

    /// Faults to enable at startup. Repeatable; faults compose.
    #[arg(long = "fault", value_name = "SPEC", action = clap::ArgAction::Append)]
    pub faults: Vec<Fault>,

    /// Path to the NVS-equivalent JSON state file.
    #[arg(long, value_name = "PATH")]
    pub state_file: Option<PathBuf>,

    /// Bind address for the simulator-only control API. Must be loopback unless
    /// the explicit E2E-only remote-control flag is also present.
    #[arg(
        long,
        value_name = "IP:PORT",
        env = "RHIZO_SIMULATOR__CONTROL_BIND",
        default_value = "127.0.0.1:9090"
    )]
    pub control_bind: SocketAddr,

    /// Permit the control API on a non-loopback address for the isolated M8
    /// Compose network. This is never enabled in the base topology.
    #[arg(long)]
    pub allow_remote_control_api: bool,

    /// Override only the control API port (backwards-compatible launch option).
    #[arg(long, value_name = "PORT")]
    pub control_port: Option<u16>,

    /// Disable the simulator-only control API entirely.
    #[arg(long)]
    pub no_control_api: bool,

    /// Write one captured example per published message kind to this directory.
    #[arg(long, value_name = "DIR")]
    pub capture_fixtures: Option<PathBuf>,

    /// Replay a scripted sequence of state changes and faults.
    #[arg(long, value_name = "FILE")]
    pub scenario: Option<PathBuf>,

    /// Exit after this many seconds of real time. Unset means run forever.
    #[arg(long, value_name = "SECONDS")]
    pub duration: Option<u64>,

    /// Seed for the deterministic sensor-noise and fault generator.
    #[arg(long, value_name = "SEED", default_value_t = 0x5eed_1234_5678_9abc)]
    pub seed: u64,

    /// Disable Gaussian sensor noise. Noise is on by default because a
    /// controller that only works on clean signals does not work.
    #[arg(long)]
    pub no_noise: bool,

    /// Power mode. `battery` sleeps between sampling cycles and announces each
    /// sleep before disconnecting (ADR-018).
    #[arg(long, value_name = "MODE", default_value = "always_on")]
    pub power_mode: PowerModeArg,

    /// How long a battery device sleeps between wakes, in seconds of virtual
    /// time. Bounded by the protocol so a device cannot announce a sleep the
    /// edge would refuse.
    #[arg(long, value_name = "SECONDS", default_value_t = 900)]
    pub wake_interval_seconds: u32,

    /// How long an *idle* wake may last, in seconds of virtual time. An active
    /// watering cycle extends it: a budget that could truncate a dose would be a
    /// way to strand an energised pump.
    #[arg(long, value_name = "SECONDS", default_value_t = 20)]
    pub awake_budget_seconds: u32,

    /// Simulated peripheral warm-up after power-on, in milliseconds of virtual
    /// time. A reading taken before it elapses is not taken at all.
    #[arg(long, value_name = "MS", default_value_t = 2_000)]
    pub sensor_warmup_ms: u32,

    /// Log format: `json`, `compact`, or `pretty`.
    #[arg(long, value_name = "FORMAT", default_value = "compact")]
    pub log_format: String,

    /// Log filter directive, `RUST_LOG` syntax.
    #[arg(long, value_name = "DIRECTIVE", default_value = "info")]
    pub log_level: String,
}

impl Cli {
    /// The effective loopback address for the simulator-only control API.
    #[must_use]
    pub fn resolved_control_bind(&self) -> SocketAddr {
        self.control_port.map_or(self.control_bind, |port| {
            SocketAddr::new(self.control_bind.ip(), port)
        })
    }

    /// The broker username, defaulting to the device id.
    #[must_use]
    pub fn resolved_username(&self) -> String {
        self.username
            .clone()
            .unwrap_or_else(|| self.device_id.to_string())
    }

    /// The environment variable holding this device's broker password.
    ///
    /// The convention `.env.example` and `scripts/gen-mosquitto-passwd.sh`
    /// already use: `RHIZO_DEVICE_<DEVICE_ID_WITH_UNDERSCORES>_PASSWORD`.
    #[must_use]
    pub fn password_env_var(&self) -> String {
        format!(
            "RHIZO_DEVICE_{}_PASSWORD",
            self.device_id.to_string().to_uppercase().replace('-', "_")
        )
    }

    /// The broker password, from `--password`, then `RHIZO_DEVICE_PASSWORD`,
    /// then the per-device variable.
    ///
    /// The per-device fallback exists because a Compose file running several
    /// simulators cannot give each container a different value under one
    /// generic name, and because the same convention already names the accounts
    /// in `deploy/mosquitto/passwd`. One convention, three consumers, no third
    /// spelling of the same secret.
    #[must_use]
    pub fn resolved_password(&self) -> Option<String> {
        self.password
            .clone()
            .or_else(|| std::env::var(self.password_env_var()).ok())
            .filter(|password| !password.is_empty())
    }

    /// The state-file path, defaulting to `<device-id>.state.json` beside the
    /// working directory.
    #[must_use]
    pub fn resolved_state_file(&self) -> PathBuf {
        self.state_file
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("{}.state.json", self.device_id)))
    }

    /// Rejects argument combinations `clap` cannot express.
    ///
    /// Range checks live here rather than in a `value_parser` so the message
    /// names the flag and the accepted range together; a bare "invalid value"
    /// tells an operator nothing about what to write instead.
    ///
    /// # Errors
    ///
    /// Returns the first violated constraint.
    pub fn validate(&self) -> Result<(), CliError> {
        if !self.resolved_control_bind().ip().is_loopback() && !self.allow_remote_control_api {
            return Err(CliError::ControlBindMustBeLoopback(
                self.resolved_control_bind(),
            ));
        }
        if !self.time_scale.is_finite() || self.time_scale <= 0.0 {
            return Err(CliError::TimeScale);
        }
        let check = |ok: bool, flag: &'static str, expected: &'static str| {
            if ok {
                Ok(())
            } else {
                Err(CliError::ModelParameter { flag, expected })
            }
        };
        check(
            self.initial_moisture.is_finite() && (0.0..=100.0).contains(&self.initial_moisture),
            "initial-moisture",
            "a percentage between 0 and 100",
        )?;
        check(
            self.drying_rate.is_finite() && self.drying_rate >= 0.0,
            "drying-rate",
            "finite and not negative",
        )?;
        check(
            self.pot_volume_ml.is_finite() && self.pot_volume_ml > 0.0,
            "pot-volume-ml",
            "greater than zero",
        )?;
        check(
            self.tank_capacity_ml.is_finite() && self.tank_capacity_ml > 0.0,
            "tank-capacity-ml",
            "greater than zero",
        )?;
        check(
            self.ml_per_second.is_finite() && (0.1..=100.0).contains(&self.ml_per_second),
            "ml-per-second",
            "between 0.1 and 100.0",
        )?;
        check(
            (10..=3600).contains(&self.telemetry_interval),
            "telemetry-interval",
            "between 10 and 3600 seconds",
        )?;
        let mut ids: Vec<&str> = self
            .actuators
            .specs()
            .iter()
            .map(|a| a.actuator_id.as_str())
            .collect();
        ids.sort_unstable();
        let unique = ids.len();
        ids.dedup();
        check(
            ids.len() == unique,
            "actuators",
            "a list of unique actuator ids",
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let mut full = vec!["device-simulator"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full)
    }

    #[test]
    fn the_minimal_invocation_supplies_every_documented_default() {
        let cli = parse(&["--device-id", "plant-node-01"]).unwrap();
        assert!(cli.validate().is_ok());
        assert_eq!(cli.broker, "mqtt://localhost:1883");
        assert_eq!(cli.resolved_username(), "plant-node-01");
        assert_eq!(cli.telemetry_interval, 300);
        assert_eq!(cli.time_scale, 1.0);
        assert_eq!(cli.log_format, "compact");
        assert_eq!(cli.resolved_control_bind().port(), DEFAULT_CONTROL_PORT);
        assert!(cli.resolved_control_bind().ip().is_loopback());
        // The default sensor list is the plant hardware. The battery gauge is
        // declared by `--power-mode battery` rather than selected here, because
        // a mains device has no pack to report.
        assert_eq!(
            cli.sensors.groups(),
            [
                SensorGroup::Soil,
                SensorGroup::Weight,
                SensorGroup::Tank,
                SensorGroup::Leak
            ]
        );
        assert_eq!(cli.power_mode, PowerModeArg::AlwaysOn);
        assert_eq!(cli.wake_interval_seconds, 900);
        assert_eq!(cli.actuators.specs().len(), 1);
        assert!(!cli.no_noise, "noise is on by default");
        assert_eq!(
            cli.resolved_state_file().file_name().unwrap(),
            "plant-node-01.state.json"
        );
    }

    #[test]
    fn the_control_api_cannot_be_exposed_beyond_loopback() {
        let cli = parse(&[
            "--device-id",
            "plant-node-01",
            "--control-bind",
            "0.0.0.0:9090",
        ])
        .unwrap();
        assert!(matches!(
            cli.validate(),
            Err(CliError::ControlBindMustBeLoopback(_))
        ));
    }

    #[test]
    fn the_e2e_flag_explicitly_allows_a_compose_network_bind() {
        let cli = parse(&[
            "--device-id",
            "plant-node-01",
            "--control-bind",
            "0.0.0.0:9090",
            "--allow-remote-control-api",
        ])
        .unwrap();
        assert!(cli.validate().is_ok());
    }

    #[test]
    fn the_per_device_password_variable_follows_the_documented_convention() {
        let cli = parse(&["--device-id", "plant-node-01"]).unwrap();
        assert_eq!(
            cli.password_env_var(),
            "RHIZO_DEVICE_PLANT_NODE_01_PASSWORD",
            "the same spelling `.env.example` and gen-mosquitto-passwd.sh use"
        );
        let cli = parse(&["--device-id", "fern-a2"]).unwrap();
        assert_eq!(cli.password_env_var(), "RHIZO_DEVICE_FERN_A2_PASSWORD");
    }

    #[test]
    fn an_explicit_password_wins_over_the_environment() {
        let cli = parse(&["--device-id", "plant-node-01", "--password", "explicit"]).unwrap();
        assert_eq!(cli.resolved_password().as_deref(), Some("explicit"));
    }

    #[test]
    fn an_empty_password_is_treated_as_absent() {
        let cli = parse(&["--device-id", "plant-node-01", "--password", ""]).unwrap();
        assert_eq!(
            cli.resolved_password(),
            None,
            "an empty variable is how a shell unsets one; honouring it literally              would send an empty password the broker would refuse"
        );
    }

    #[test]
    fn a_missing_device_id_is_an_error_not_a_default() {
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn an_invalid_device_id_is_rejected_by_the_shared_grammar() {
        // The simulator does not re-implement the grammar; `DeviceId::parse`
        // is the one definition (ADR-012).
        assert!(parse(&["--device-id", "Plant-01"]).is_err());
        assert!(parse(&["--device-id", "x/#"]).is_err());
    }

    #[test]
    fn sensor_and_actuator_lists_may_be_empty() {
        let cli = parse(&[
            "--device-id",
            "plant-node-01",
            "--sensors",
            "",
            "--actuators",
            "",
        ])
        .unwrap();
        assert!(cli.sensors.groups().is_empty());
        assert!(
            cli.actuators.specs().is_empty(),
            "a monitoring-only device is normal, not degraded (SAFETY-018)"
        );
        assert!(cli.validate().is_ok());
    }

    #[test]
    fn sensor_groups_parse_from_a_comma_list() {
        let cli = parse(&["--device-id", "plant-node-01", "--sensors", "soil,tank"]).unwrap();
        assert_eq!(
            cli.sensors.groups(),
            [SensorGroup::Soil, SensorGroup::Tank].as_slice()
        );
        assert!(parse(&["--device-id", "plant-node-01", "--sensors", "moon"]).is_err());
    }

    #[test]
    fn actuator_kind_defaults_to_irrigation_pump_and_may_be_named() {
        let cli = parse(&[
            "--device-id",
            "plant-node-01",
            "--actuators",
            "pump-0,valve-1:valve",
        ])
        .unwrap();
        assert_eq!(cli.actuators.specs()[0].kind, ActuatorKind::IrrigationPump);
        assert_eq!(cli.actuators.specs()[1].kind, ActuatorKind::Valve);
        assert!(
            parse(&[
                "--device-id",
                "plant-node-01",
                "--actuators",
                "pump-0:teleporter",
            ])
            .is_err()
        );
    }

    #[test]
    fn duplicate_actuator_ids_are_rejected() {
        let cli = parse(&[
            "--device-id",
            "plant-node-01",
            "--actuators",
            "pump-0,pump-0",
        ])
        .unwrap();
        assert!(matches!(
            cli.validate(),
            Err(CliError::ModelParameter {
                flag: "actuators",
                ..
            })
        ));
    }

    #[test]
    fn every_catalogued_fault_parses_and_round_trips() {
        let cases = [
            ("disconnect:30", Fault::Disconnect { seconds: 30 }),
            ("duplicate:0.5", Fault::Duplicate { rate: 0.5 }),
            ("reorder:0.25", Fault::Reorder { rate: 0.25 }),
            ("invalid-soil:1", Fault::InvalidSoil { rate: 1.0 }),
            ("stale-soil", Fault::StaleSoil),
            ("stale-tank", Fault::StaleTank),
            ("stale-leak", Fault::StaleLeak),
            ("stale-weight", Fault::StaleWeight),
            ("stuck-sensor", Fault::StuckSensor),
            ("clock-unsync", Fault::ClockUnsync),
            ("clock-skew:-90", Fault::ClockSkew { seconds: -90 }),
            ("leak", Fault::Leak),
            ("tank-empty", Fault::TankEmpty),
            ("pump-no-delivery", Fault::PumpNoDelivery),
            ("pump-stuck-on", Fault::PumpStuckOn),
            ("restart-mid-dose", Fault::RestartMidDose),
            ("disconnect-mid-dose", Fault::DisconnectMidDose),
            ("restart", Fault::Restart),
            (
                "policy-interrupt:stage",
                Fault::PolicyInterrupt {
                    step: PolicyStep::Stage,
                },
            ),
            ("miss-wake:2", Fault::MissWake { count: 2 }),
            ("sleep-without-announcing", Fault::SleepWithoutAnnouncing),
        ];
        assert_eq!(
            cases.len(),
            Fault::NAMES.len(),
            "the catalogue and its tests must not drift"
        );
        for (spec, expected) in cases {
            assert_eq!(spec.parse::<Fault>().unwrap(), expected, "{spec}");
            assert_eq!(expected.to_string().parse::<Fault>().unwrap(), expected);
        }
    }

    #[test]
    fn malformed_fault_specifications_are_rejected_with_a_reason() {
        assert_eq!(
            "teleport".parse::<Fault>(),
            Err(CliError::UnknownFault("teleport".into()))
        );
        assert_eq!(
            "disconnect".parse::<Fault>(),
            Err(CliError::FaultNeedsArgument("disconnect".into()))
        );
        assert_eq!(
            "duplicate:2.0".parse::<Fault>(),
            Err(CliError::FaultRateRange("duplicate:2.0".into()))
        );
        assert_eq!(
            "duplicate:-0.1".parse::<Fault>(),
            Err(CliError::FaultRateRange("duplicate:-0.1".into()))
        );
        assert_eq!(
            "duplicate:soon".parse::<Fault>(),
            Err(CliError::FaultArgument("duplicate:soon".into()))
        );
        assert_eq!(
            "policy-interrupt:later".parse::<Fault>(),
            Err(CliError::UnknownPolicyStep("later".into()))
        );
    }

    #[test]
    fn faults_are_repeatable_and_compose() {
        let cli = parse(&[
            "--device-id",
            "plant-node-01",
            "--fault",
            "leak",
            "--fault",
            "tank-empty",
        ])
        .unwrap();
        assert_eq!(cli.faults, vec![Fault::Leak, Fault::TankEmpty]);
    }

    #[test]
    fn out_of_range_model_parameters_are_rejected_by_validate() {
        let cases: [&[&str]; 6] = [
            &["--time-scale", "0"],
            &["--initial-moisture", "101"],
            // `=` form: a bare `-1` would be read as a flag, not a value.
            &["--drying-rate=-1"],
            &["--pot-volume-ml", "0"],
            &["--ml-per-second", "0.01"],
            &["--telemetry-interval", "9"],
        ];
        for extra in cases {
            let mut args = vec!["--device-id", "plant-node-01"];
            args.extend_from_slice(extra);
            let cli = parse(&args).unwrap();
            assert!(cli.validate().is_err(), "{extra:?} should be rejected");
        }
    }

    #[test]
    fn help_lists_every_documented_flag() {
        let help = Cli::try_parse_from(["device-simulator", "--help"])
            .unwrap_err()
            .to_string();
        for flag in [
            "--device-id",
            "--broker",
            "--initial-moisture",
            "--drying-rate",
            "--pot-volume-ml",
            "--tank-capacity-ml",
            "--ml-per-second",
            "--telemetry-interval",
            "--time-scale",
            "--sensors",
            "--actuators",
            "--fault",
            "--scenario",
            "--capture-fixtures",
            "--control-port",
            "--state-file",
            "--seed",
        ] {
            assert!(help.contains(flag), "`--help` does not mention {flag}");
        }
    }
}
