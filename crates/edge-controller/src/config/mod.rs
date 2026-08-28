//! Edge instance configuration — configuration layer L2.
//!
//! ```text
//! 1. built-in defaults          (DEFAULTS_TOML, below)
//! 2. edge.toml                  (--config, or ./edge.toml, or /etc/rhizo/edge.toml)
//! 3. RHIZO_EDGE__* environment  (`__` separates nesting)
//! 4. command-line flags         (--log-level)
//! ```
//!
//! Later layers win. The normative table is in
//! [configuration-model.md](../../../../docs/architecture/configuration-model.md)
//! §L2; the reasoning is in
//! [ADR-011](../../../../docs/adr/011-configuration-and-secrets-model.md).
//!
//! # Fail fast
//!
//! [`load`] validates the whole configuration and returns an error naming the
//! offending key rather than substituting a default. An edge that starts with
//! a silently-wrong value is worse than one that refuses to start: the
//! operator believes something false about their system and finds out during
//! an incident.
//!
//! # Secrets are not readable from the file
//!
//! `mqtt.password` exists **only** in the environment layer. A `password` key
//! written into `edge.toml` is stripped before the file is merged, and the
//! attempt is reported as a warning. Config files get pasted into bug reports;
//! environment variables do not.
//!
//! # Why `figment`
//!
//! ADR-011 left the choice between `figment` and `config` to this issue,
//! deciding on the quality of the error for a malformed key. Both name the
//! key. The comparison, run against a `tick_interval_seconds = "thirty"`:
//!
//! ```text
//! figment: invalid type: found string "thirty", expected u64
//!          for key "control.tick_interval_seconds" in TOML source string
//!          + Error::path == ["control", "tick_interval_seconds"]
//!          + Error::metadata.name == "TOML source string"
//!
//! config:  invalid type: string "thirty", expected an integer
//!          for key `control.tick_interval_seconds`
//!          (Debug output is identical to Display; no structured path)
//! ```
//!
//! `figment` was chosen for two things `config` does not offer:
//!
//! 1. **A structured key path.** [`ConfigError::key`] is built from
//!    `Error::path`, not by scraping prose, so "exits non-zero naming the key"
//!    keeps working when an upstream message is reworded.
//! 2. **Layer attribution.** The message names *which* layer supplied the bad
//!    value. With four layers able to set the same key, that is usually the
//!    whole answer — and `figment` reports it for environment variables too,
//!    not only files.
//!
//! The environment layer is a small local provider ([`layers::PrefixedEnv`])
//! rather than `figment::providers::Env`, for one reason: `Env` reads the real
//! process environment, which would make every precedence test depend on the
//! developer's shell and require mutating the environment from a test — both
//! `unsafe` in edition 2024 and unsound under parallel test execution. It
//! reproduces `Env`'s prefix filtering, `__` nesting, and value typing, and
//! adds the same layer naming to its metadata.

mod error;
mod layers;
mod secret;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use figment::providers::{Format, Toml};
use figment::value::{Dict, Tag, Value};
use figment::{Figment, Profile};
use serde::Deserialize;

use layers::{NamedDict, PrefixedEnv};

pub use error::ConfigError;
pub use secret::Secret;

/// The environment-variable prefix for layer 3.
pub const ENV_PREFIX: &str = "RHIZO_EDGE__";

/// The separator for nested keys in environment variables.
///
/// `RHIZO_EDGE__MQTT__BROKER_URL` reaches `config.mqtt.broker_url`.
pub const ENV_NESTING_SEPARATOR: &str = "__";

/// Where the file layer is looked for when `--config` is not given.
///
/// The working-directory copy first so a developer's local file wins over an
/// installed one; neither has to exist.
pub const DEFAULT_CONFIG_PATHS: [&str; 2] = ["edge.toml", "/etc/rhizo/edge.toml"];

/// Substrings that mark a key as secret-shaped.
///
/// Matched case-insensitively against the *leaf* key name. Deliberately a
/// short, blunt list: the cost of a false positive is a warning and an ignored
/// key, and the cost of a false negative is a credential in a bug report.
const SECRET_KEY_MARKERS: [&str; 3] = ["password", "token", "secret"];

/// The built-in defaults, layer 1.
///
/// Written as TOML rather than as a `Default` impl so that the defaults and
/// the file an operator writes have exactly the same shape, and so this
/// listing can be read as documentation of what a complete configuration
/// contains.
///
/// `cloud.enabled = false` and `api.bind = 127.0.0.1:8080` are the two that
/// carry weight: the cloud is opt-in, and the API listens on loopback until
/// somebody explicitly widens it (ADR-011 — V1 has no authentication on the
/// edge API, so its reachability is the whole access control story).
pub const DEFAULTS_TOML: &str = r#"
edge_id = "home-01"

[mqtt]
broker_url = "mqtt://localhost:1883"
client_id  = "rhizo-edge-home-01"
username   = "rhizo-edge"
password   = ""

[storage]
path = "./data/edge.sqlite"

[control]
tick_interval_seconds = 30
command_ttl_seconds   = 120

[cloud]
enabled  = false
base_url = "http://localhost:8081"

[api]
bind = "127.0.0.1:8080"
cors_allowed_origins = []

[log]
level  = "info"
format = "json"
"#;

/// The edge's instance configuration.
///
/// `Debug` is derived, and is safe to print: every secret-shaped field is a
/// [`Secret`], which redacts itself.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EdgeConfig {
    /// Stable identity for this edge, used as the cloud partition key.
    ///
    /// Changing it orphans this edge's history in the cloud, so it is treated
    /// as permanent once a deployment exists.
    pub edge_id: String,
    /// Broker connection settings.
    pub mqtt: MqttConfig,
    /// Local persistence settings.
    pub storage: StorageConfig,
    /// Control-loop timing.
    pub control: ControlConfig,
    /// Optional cloud replication.
    pub cloud: CloudConfig,
    /// The edge's own HTTP surface.
    pub api: ApiConfig,
    /// Logging.
    pub log: LogConfig,
}

/// MQTT broker connection settings.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MqttConfig {
    /// `mqtt://host[:port]`. TLS (`mqtts://`) is a M13 deliverable.
    pub broker_url: String,
    /// The MQTT client identifier this edge connects with.
    pub client_id: String,
    /// The broker account. ADR-012 gives the edge a broad account
    /// (`readwrite rhizo/v1/#`) distinct from every device's.
    pub username: String,
    /// The broker password.
    ///
    /// Settable **only** through `RHIZO_EDGE__MQTT__PASSWORD`. A `password`
    /// key in `edge.toml` is stripped and warned about — see the module docs.
    pub password: Secret,
}

/// Local persistence settings.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    /// Path to the SQLite database file. The schema itself is an M3
    /// deliverable; M0 only validates that a path was configured.
    pub path: PathBuf,
}

/// Control-loop timing.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ControlConfig {
    /// How often the control loop evaluates every plant.
    pub tick_interval_seconds: u64,
    /// How long an issued watering command stays valid (SAFETY-002).
    pub command_ttl_seconds: u64,
}

/// Optional cloud replication.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CloudConfig {
    /// Whether to replicate history to the cloud. Defaults to `false`:
    /// the cloud is opt-in, and its absence is the normal case (SAFETY-008).
    pub enabled: bool,
    /// Base URL of the cloud API. Validated only when `enabled`.
    pub base_url: String,
}

/// The edge's own HTTP surface.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
    /// Address to bind. Loopback by default; widening it is explicit, because
    /// V1 has no authentication on this API.
    pub bind: SocketAddr,
    /// Exact origins allowed to use the browser API. Wildcards are forbidden.
    #[serde(default)]
    pub cors_allowed_origins: Vec<String>,
}

/// Logging.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LogConfig {
    /// A `RUST_LOG`-compatible directive, e.g. `info` or
    /// `info,rhizo_storage=debug`.
    pub level: String,
    /// `json` or `pretty`.
    pub format: String,
}

impl LogConfig {
    /// The parsed log format.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Invalid`] naming `log.format`.
    pub fn parsed_format(&self) -> Result<rhizo_telemetry::LogFormat, ConfigError> {
        self.format.parse().map_err(|_| {
            ConfigError::invalid(
                "log.format",
                format!(
                    "`{}` is not a log format (accepted: {})",
                    self.format,
                    rhizo_telemetry::LogFormat::ACCEPTED.join(", ")
                ),
            )
        })
    }
}

/// Command-line overrides, the last layer.
///
/// Deliberately tiny. ADR-011 §L2 allows "a small set of CLI flags"; anything
/// larger belongs in the file or the environment, where it can be commented
/// and version-controlled.
#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    /// An explicit configuration file. When set and missing, loading fails —
    /// an operator who names a file meant it.
    pub config_path: Option<PathBuf>,
    /// Overrides `log.level`.
    pub log_level: Option<String>,
}

/// A successfully loaded configuration, plus anything the operator should know.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// The validated configuration.
    pub config: EdgeConfig,
    /// Warnings raised while loading.
    ///
    /// Returned rather than logged because the log format is itself
    /// configuration: the subscriber does not exist yet when these are
    /// produced. The caller emits them immediately after initialising
    /// tracing — see [`Loaded::emit_warnings`].
    pub warnings: Vec<Warning>,
    /// The file that was actually merged, if any.
    pub config_file: Option<PathBuf>,
}

/// Something worth telling the operator that is not fatal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// A secret-shaped key was found in the file layer and ignored.
    SecretInFile {
        /// The dotted path of the offending key.
        key: String,
    },
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SecretInFile { key } => write!(
                f,
                "ignoring `{key}` from the configuration file: secrets are read \
                 only from the environment (set {ENV_PREFIX}{env}) — a config \
                 file gets pasted into bug reports",
                env = key.replace('.', ENV_NESTING_SEPARATOR).to_uppercase()
            ),
        }
    }
}

impl Loaded {
    /// Logs every warning at WARN.
    ///
    /// Call once, immediately after the tracing subscriber is installed.
    pub fn emit_warnings(&self) {
        for w in &self.warnings {
            match w {
                Warning::SecretInFile { key } => {
                    tracing::warn!(config_key = %key, "{w}");
                }
            }
        }
    }
}

/// Loads, layers, and validates the configuration.
///
/// # Errors
///
/// Returns a [`ConfigError`] naming the offending key. Every error from this
/// function is [`FailureKind::Fatal`](rhizo_telemetry::FailureKind::Fatal) —
/// the caller's only correct response is to report it and exit non-zero.
pub fn load(cli: &CliOverrides) -> Result<Loaded, ConfigError> {
    load_from_env(cli, std::env::vars())
}

/// [`load`], with the environment supplied explicitly.
///
/// The seam that keeps the layering tests hermetic: reading the real process
/// environment would make them depend on the developer's shell, and setting
/// variables from a test is both `unsafe` in edition 2024 and unsound when
/// tests run in parallel.
///
/// # Errors
///
/// As [`load`].
pub fn load_from_env<I>(cli: &CliOverrides, env: I) -> Result<Loaded, ConfigError>
where
    I: IntoIterator<Item = (String, String)>,
{
    let config_file = resolve_file(cli)?;

    let mut warnings = Vec::new();
    let mut figment = Figment::from(Toml::string(DEFAULTS_TOML));

    if let Some(path) = &config_file {
        let dict = read_file_layer(path)?;
        let sanitised = strip_secrets(dict, "", &mut warnings);
        figment = figment.merge(NamedDict::new(
            format!("configuration file `{}`", path.display()),
            sanitised,
        ));
    }

    figment = figment.merge(PrefixedEnv::new(ENV_PREFIX, ENV_NESTING_SEPARATOR, env));

    if let Some(level) = &cli.log_level {
        let mut log = Dict::new();
        log.insert(String::from("level"), Value::from(level.as_str()));
        let mut root = Dict::new();
        root.insert(String::from("log"), Value::from(log));
        figment = figment.merge(NamedDict::new("command-line flag `--log-level`", root));
    }

    let config: EdgeConfig = figment.extract().map_err(to_config_error)?;
    validate(&config)?;

    Ok(Loaded {
        config,
        warnings,
        config_file,
    })
}

/// Decides which file, if any, forms the file layer.
fn resolve_file(cli: &CliOverrides) -> Result<Option<PathBuf>, ConfigError> {
    if let Some(explicit) = &cli.config_path {
        if !explicit.is_file() {
            // An operator who names a file meant it. Falling back to defaults
            // here is the exact "started with something other than what you
            // asked for" failure ADR-011 forbids.
            return Err(ConfigError::FileNotFound {
                path: explicit.display().to_string(),
            });
        }
        return Ok(Some(explicit.clone()));
    }
    for candidate in DEFAULT_CONFIG_PATHS {
        let p = Path::new(candidate);
        if p.is_file() {
            return Ok(Some(p.to_path_buf()));
        }
    }
    // No file is not an error: PRD 000 requires defaults plus environment to
    // be a complete, working configuration.
    Ok(None)
}

/// Parses the file layer into a dictionary so secrets can be removed from it
/// before it is merged.
fn read_file_layer(path: &Path) -> Result<Dict, ConfigError> {
    Figment::from(Toml::file(path))
        .select(Profile::Default)
        .extract()
        .map_err(to_config_error)
}

/// Removes secret-shaped keys, recording one warning each.
///
/// Recursive, because `[mqtt] password = ...` is nested and a top-level-only
/// check would miss the exact case that matters.
fn strip_secrets(dict: Dict, prefix: &str, warnings: &mut Vec<Warning>) -> Dict {
    let mut out = Dict::new();
    for (key, value) in dict {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };

        if is_secret_key(&key) {
            warnings.push(Warning::SecretInFile { key: path });
            continue;
        }

        let value = match value {
            Value::Dict(_, inner) => {
                Value::Dict(Tag::Default, strip_secrets(inner, &path, warnings))
            }
            other => untag(other),
        };
        out.insert(key, value);
    }
    out
}

/// Clears the `figment` tag on a value.
///
/// Values extracted out of one `Figment` carry tags pointing at *that*
/// figment's metadata. Merged into a new one, those tags resolve to nothing,
/// and a later error reports its layer as "configuration" instead of naming
/// the file. Clearing them lets the receiving figment attach the metadata of
/// the provider it is merging — which is the whole reason this layer is a
/// named provider.
fn untag(value: Value) -> Value {
    match value {
        Value::String(_, v) => Value::String(Tag::Default, v),
        Value::Char(_, v) => Value::Char(Tag::Default, v),
        Value::Bool(_, v) => Value::Bool(Tag::Default, v),
        Value::Num(_, v) => Value::Num(Tag::Default, v),
        Value::Empty(_, v) => Value::Empty(Tag::Default, v),
        Value::Array(_, v) => Value::Array(Tag::Default, v.into_iter().map(untag).collect()),
        Value::Dict(_, v) => Value::Dict(
            Tag::Default,
            v.into_iter().map(|(k, val)| (k, untag(val))).collect(),
        ),
    }
}

/// Whether a leaf key name looks like it holds a credential.
fn is_secret_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    SECRET_KEY_MARKERS.iter().any(|m| lowered.contains(m))
}

/// Converts a `figment` error into one that names the key and the layer.
///
/// Uses the structured `path` and `metadata`, not the rendered message, so
/// this keeps working if upstream rewords anything.
fn to_config_error(err: figment::Error) -> ConfigError {
    let first = err.clone().into_iter().next();
    let (key, source) = first.map_or_else(
        || (String::from("(unknown)"), String::from("configuration")),
        |e| {
            let key = if e.path.is_empty() {
                String::from("(root)")
            } else {
                e.path.join(".")
            };
            let source = e
                .metadata
                .as_ref()
                .map_or_else(|| String::from("configuration"), |m| m.name.to_string());
            (key, source)
        },
    );
    ConfigError::Malformed {
        key,
        layer: source,
        detail: err.to_string(),
    }
}

/// Rejects values that parsed but cannot be used.
///
/// Every check names its key. The bar for adding one is that the value is
/// *unusable*, not merely unusual — refusing to start is a strong response and
/// is reserved for configuration that could not do the right thing.
fn validate(c: &EdgeConfig) -> Result<(), ConfigError> {
    validate_identifier("edge_id", &c.edge_id)?;

    validate_broker_url(&c.mqtt.broker_url)?;
    if c.mqtt.client_id.trim().is_empty() {
        return Err(ConfigError::invalid(
            "mqtt.client_id",
            "must not be empty — the broker uses it to identify this session",
        ));
    }
    if c.mqtt.username.trim().is_empty() {
        return Err(ConfigError::invalid(
            "mqtt.username",
            "must not be empty — anonymous access is disabled on the broker (ADR-012)",
        ));
    }

    if c.storage.path.as_os_str().is_empty() {
        return Err(ConfigError::invalid("storage.path", "must not be empty"));
    }

    if c.control.tick_interval_seconds == 0 {
        return Err(ConfigError::invalid(
            "control.tick_interval_seconds",
            "must be greater than zero — a zero interval is a busy loop, not a fast one",
        ));
    }
    if c.control.command_ttl_seconds == 0 {
        return Err(ConfigError::invalid(
            "control.command_ttl_seconds",
            "must be greater than zero — a zero TTL expires every command before it can be acted on",
        ));
    }
    if c.control.command_ttl_seconds < c.control.tick_interval_seconds {
        return Err(ConfigError::invalid(
            "control.command_ttl_seconds",
            format!(
                "must be at least control.tick_interval_seconds ({}), otherwise a command expires \
                 before the loop that issued it next runs; got {}",
                c.control.tick_interval_seconds, c.control.command_ttl_seconds
            ),
        ));
    }

    if c.cloud.enabled {
        validate_http_url("cloud.base_url", &c.cloud.base_url)?;
    }
    if c.api
        .cors_allowed_origins
        .iter()
        .any(|origin| origin == "*")
    {
        return Err(ConfigError::invalid(
            "api.cors_allowed_origins",
            "wildcard origins are not permitted",
        ));
    }

    rhizo_telemetry::validate_filter(&c.log.level)
        .map_err(|e| ConfigError::invalid("log.level", e.to_string()))?;
    c.log.parsed_format()?;

    Ok(())
}

/// Validates an identifier that will appear in topics, rows, and URLs.
///
/// The same restrictions as ADR-012's `device_id` grammar, for the same
/// reason: barring `+`, `#`, `/`, and whitespace is what prevents an
/// identifier from breaking out of the topic subtree it belongs to, and
/// lowercase-only removes any disagreement between MQTT topic matching and
/// database collation about whether `Home-01` and `home-01` are the same
/// thing.
fn validate_identifier(key: &str, value: &str) -> Result<(), ConfigError> {
    let ok = (3..=32).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && value
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());

    if ok {
        Ok(())
    } else {
        Err(ConfigError::invalid(
            key,
            format!(
                "`{value}` is not a valid identifier: 3-32 characters of lowercase letters, \
                 digits, and hyphens, starting and ending alphanumeric"
            ),
        ))
    }
}

/// Validates the broker URL's scheme and authority.
fn validate_broker_url(value: &str) -> Result<(), ConfigError> {
    const KEY: &str = "mqtt.broker_url";
    let Some((scheme, rest)) = value.split_once("://") else {
        return Err(ConfigError::invalid(
            KEY,
            format!("`{value}` has no scheme; expected `mqtt://host[:port]`"),
        ));
    };
    if !matches!(scheme, "mqtt" | "mqtts") {
        return Err(ConfigError::invalid(
            KEY,
            format!("unsupported scheme `{scheme}`; expected `mqtt` or `mqtts`"),
        ));
    }
    let host = rest.split(':').next().unwrap_or_default();
    if host.is_empty() {
        return Err(ConfigError::invalid(
            KEY,
            format!("`{value}` has no host; expected `{scheme}://host[:port]`"),
        ));
    }
    if let Some((_, port)) = rest.split_once(':') {
        let port = port.trim_end_matches('/');
        if port.parse::<u16>().is_err() {
            return Err(ConfigError::invalid(
                KEY,
                format!("`{port}` is not a valid TCP port"),
            ));
        }
    }
    Ok(())
}

/// Validates an HTTP(S) URL's scheme and authority.
fn validate_http_url(key: &str, value: &str) -> Result<(), ConfigError> {
    let Some((scheme, rest)) = value.split_once("://") else {
        return Err(ConfigError::invalid(
            key,
            format!("`{value}` has no scheme; expected `http://host[:port]`"),
        ));
    };
    if !matches!(scheme, "http" | "https") {
        return Err(ConfigError::invalid(
            key,
            format!("unsupported scheme `{scheme}`; expected `http` or `https`"),
        ));
    }
    if rest.split(['/', ':']).next().unwrap_or_default().is_empty() {
        return Err(ConfigError::invalid(key, format!("`{value}` has no host")));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
