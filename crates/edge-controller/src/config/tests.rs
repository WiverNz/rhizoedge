//! Configuration tests (M0-005).
//!
//! Every test supplies its environment explicitly through
//! [`load_from_env`](super::load_from_env), so none of them depends on the
//! developer's shell and all of them can run in parallel.

use std::io::Write;

use rhizo_telemetry::{Classify, FailureKind};

use super::*;

/// A `.toml` file that deletes itself.
///
/// Small enough to be worth writing rather than adding a temp-file dependency
/// for four tests.
struct TempToml {
    path: PathBuf,
}

impl TempToml {
    fn new(contents: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "rhizo-edge-config-{}-{:?}.toml",
            std::process::id(),
            std::thread::current().id()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        Self { path }
    }

    fn cli(&self) -> CliOverrides {
        CliOverrides {
            config_path: Some(self.path.clone()),
            log_level: None,
        }
    }
}

impl Drop for TempToml {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// Loading with no file and no environment — the pure-defaults path.
fn defaults() -> EdgeConfig {
    load_from_env(&CliOverrides::default(), env(&[]))
        .expect("the built-in defaults must be a valid configuration")
        .config
}

// ------------------------------------------------------------------ defaults

#[test]
fn defaults_load_with_no_file_and_no_environment() {
    let c = defaults();
    assert_eq!(c.edge_id, "home-01");
    assert_eq!(c.mqtt.broker_url, "mqtt://localhost:1883");
    assert_eq!(c.mqtt.username, "rhizo-edge");
    assert_eq!(c.control.tick_interval_seconds, 30);
    assert_eq!(c.control.command_ttl_seconds, 120);
    assert_eq!(c.log.level, "info");
    assert_eq!(c.log.format, "json");
}

#[test]
fn cloud_is_disabled_by_default() {
    // The cloud is opt-in. An edge that silently started replicating to a
    // default endpoint would be a surprise of exactly the wrong kind.
    assert!(!defaults().cloud.enabled);
}

#[test]
fn the_api_binds_to_loopback_by_default() {
    // V1 has no authentication on the edge API, so its reachability is the
    // whole access-control story (ADR-011). Widening it must be deliberate.
    assert_eq!(defaults().api.bind.to_string(), "127.0.0.1:8080");
    assert!(defaults().api.bind.ip().is_loopback());
}

#[test]
fn the_default_password_is_empty_and_still_redacts() {
    let c = defaults();
    assert!(c.mqtt.password.is_empty());
    assert_eq!(format!("{:?}", c.mqtt.password), Secret::REDACTED);
}

// ---------------------------------------------------------------- precedence

#[test]
fn a_file_overrides_the_defaults() {
    let f = TempToml::new(
        r#"
edge_id = "greenhouse-a"
[control]
tick_interval_seconds = 15
command_ttl_seconds = 60
"#,
    );
    let c = load_from_env(&f.cli(), env(&[])).unwrap().config;
    assert_eq!(c.edge_id, "greenhouse-a");
    assert_eq!(c.control.tick_interval_seconds, 15);
    // Untouched keys keep their default.
    assert_eq!(c.mqtt.username, "rhizo-edge");
}

#[test]
fn the_environment_overrides_the_file() {
    let f = TempToml::new(
        r#"
edge_id = "from-file"
[mqtt]
broker_url = "mqtt://from-file:1883"
"#,
    );
    let c = load_from_env(
        &f.cli(),
        env(&[
            ("RHIZO_EDGE__EDGE_ID", "from-env"),
            ("RHIZO_EDGE__MQTT__BROKER_URL", "mqtt://from-env:1883"),
        ]),
    )
    .unwrap()
    .config;
    assert_eq!(c.edge_id, "from-env");
    assert_eq!(c.mqtt.broker_url, "mqtt://from-env:1883");
}

#[test]
fn a_flag_overrides_the_environment() {
    let cli = CliOverrides {
        config_path: None,
        log_level: Some(String::from("trace")),
    };
    let c = load_from_env(&cli, env(&[("RHIZO_EDGE__LOG__LEVEL", "warn")]))
        .unwrap()
        .config;
    assert_eq!(c.log.level, "trace");
}

#[test]
fn the_environment_overrides_the_defaults_with_no_file_present() {
    let c = load_from_env(
        &CliOverrides::default(),
        env(&[("RHIZO_EDGE__CONTROL__TICK_INTERVAL_SECONDS", "45")]),
    )
    .unwrap()
    .config;
    assert_eq!(c.control.tick_interval_seconds, 45);
}

#[test]
fn a_double_underscore_variable_reaches_the_nested_key() {
    // The acceptance criterion, stated literally.
    let c = load_from_env(
        &CliOverrides::default(),
        env(&[("RHIZO_EDGE__MQTT__BROKER_URL", "mqtt://broker.local:8883")]),
    )
    .unwrap()
    .config;
    assert_eq!(c.mqtt.broker_url, "mqtt://broker.local:8883");
}

#[test]
fn environment_values_are_typed_not_left_as_strings() {
    let c = load_from_env(
        &CliOverrides::default(),
        env(&[
            ("RHIZO_EDGE__CLOUD__ENABLED", "true"),
            ("RHIZO_EDGE__CONTROL__TICK_INTERVAL_SECONDS", "60"),
        ]),
    )
    .unwrap()
    .config;
    assert!(c.cloud.enabled);
    assert_eq!(c.control.tick_interval_seconds, 60);
}

#[test]
fn variables_for_other_components_are_ignored() {
    let c = load_from_env(
        &CliOverrides::default(),
        env(&[
            ("RHIZO_SIM__TIME_SCALE", "600"),
            ("PATH", "/usr/bin"),
            ("RHIZO_EDGE__EDGE_ID", "home-02"),
        ]),
    )
    .unwrap()
    .config;
    assert_eq!(c.edge_id, "home-02");
}

// ------------------------------------------------------------------- secrets

#[test]
fn the_mqtt_password_is_read_from_the_environment() {
    let c = load_from_env(
        &CliOverrides::default(),
        env(&[("RHIZO_EDGE__MQTT__PASSWORD", "from-the-environment")]),
    )
    .unwrap()
    .config;
    assert_eq!(c.mqtt.password.expose(), "from-the-environment");
}

#[test]
fn a_password_in_the_file_is_ignored_and_warned_about() {
    let f = TempToml::new(
        r#"
[mqtt]
broker_url = "mqtt://localhost:1883"
password = "written-in-the-config-file"
"#,
    );
    let loaded = load_from_env(&f.cli(), env(&[])).unwrap();

    assert!(
        loaded.config.mqtt.password.is_empty(),
        "the file value must not be honoured"
    );
    assert_eq!(
        loaded.warnings,
        vec![Warning::SecretInFile {
            key: String::from("mqtt.password")
        }]
    );

    let rendered = loaded.warnings[0].to_string();
    assert!(rendered.contains("mqtt.password"), "{rendered}");
    assert!(
        rendered.contains("RHIZO_EDGE__MQTT__PASSWORD"),
        "the warning must say where the value belongs instead: {rendered}"
    );
    assert!(
        !rendered.contains("written-in-the-config-file"),
        "the warning must not quote the secret: {rendered}"
    );
}

#[test]
fn a_password_in_the_file_does_not_override_the_environment() {
    // The dangerous ordering: the file layer is merged after the defaults and
    // before the environment, so an unsanitised file value would still lose —
    // but a file value with no environment set would win. This asserts the
    // file value is gone, not merely outranked.
    let f = TempToml::new(
        r#"
[mqtt]
password = "file-value"
"#,
    );
    let loaded = load_from_env(
        &f.cli(),
        env(&[("RHIZO_EDGE__MQTT__PASSWORD", "env-value")]),
    )
    .unwrap();
    assert_eq!(loaded.config.mqtt.password.expose(), "env-value");
    assert_eq!(loaded.warnings.len(), 1);
}

#[test]
fn every_secret_shaped_key_in_the_file_is_stripped() {
    let f = TempToml::new(
        r#"
[mqtt]
password = "a"
api_token = "b"
client_secret = "c"
username = "rhizo-edge"
"#,
    );
    let loaded = load_from_env(&f.cli(), env(&[])).unwrap();
    let keys: Vec<_> = loaded
        .warnings
        .iter()
        .map(|w| match w {
            Warning::SecretInFile { key } => key.clone(),
        })
        .collect();
    assert!(keys.contains(&String::from("mqtt.password")));
    assert!(keys.contains(&String::from("mqtt.api_token")));
    assert!(keys.contains(&String::from("mqtt.client_secret")));
    // The non-secret sibling survives — proving the filter is not just
    // discarding the table.
    assert_eq!(loaded.config.mqtt.username, "rhizo-edge");
}

#[test]
fn secret_key_matching_is_case_insensitive() {
    assert!(is_secret_key("PASSWORD"));
    assert!(is_secret_key("Api_Token"));
    assert!(is_secret_key("clientSecret"));
    assert!(!is_secret_key("username"));
    assert!(!is_secret_key("broker_url"));
}

// ----------------------------------------------------------------- redaction

#[test]
fn debug_on_the_whole_config_redacts_the_password() {
    let c = load_from_env(
        &CliOverrides::default(),
        env(&[("RHIZO_EDGE__MQTT__PASSWORD", "super-secret-value")]),
    )
    .unwrap()
    .config;

    let rendered = format!("{c:?}");
    assert!(
        rendered.contains(Secret::REDACTED),
        "the placeholder must appear: {rendered}"
    );
    assert!(
        !rendered.contains("super-secret-value"),
        "the secret must not appear: {rendered}"
    );
    // Non-secret fields are still legible — a config that redacted everything
    // would just be a different way of being useless in a bug report.
    assert!(rendered.contains("home-01"));
    assert!(rendered.contains("mqtt://localhost:1883"));
}

// ---------------------------------------------------------------- fail fast

/// Asserts that a configuration is rejected, naming `key`.
fn assert_rejected(cli: &CliOverrides, vars: &[(&str, &str)], key: &str) {
    let err = load_from_env(cli, env(vars)).expect_err("this configuration must be rejected");
    assert_eq!(err.key(), key, "wrong key reported: {err}");
    assert!(
        err.to_string().contains(key),
        "the message must name the key: {err}"
    );
    assert_eq!(
        err.classify(),
        FailureKind::Fatal,
        "invalid configuration is Fatal (ADR-014)"
    );
}

#[test]
fn a_missing_explicit_config_file_is_an_error() {
    let cli = CliOverrides {
        config_path: Some(PathBuf::from("/nonexistent/bad.toml")),
        log_level: None,
    };
    let err = load_from_env(&cli, env(&[])).unwrap_err();
    assert!(matches!(err, ConfigError::FileNotFound { .. }));
    assert!(err.to_string().contains("bad.toml"), "{err}");
}

#[test]
fn a_malformed_value_names_the_key_and_the_layer_it_came_from() {
    let f = TempToml::new(
        r#"
[control]
tick_interval_seconds = "thirty"
"#,
    );
    let err = load_from_env(&f.cli(), env(&[])).unwrap_err();
    assert_eq!(err.key(), "control.tick_interval_seconds");
    let msg = err.to_string();
    assert!(msg.contains("control.tick_interval_seconds"), "{msg}");
    assert!(
        msg.contains("configuration file"),
        "the message must say which layer supplied it: {msg}"
    );
}

#[test]
fn a_malformed_environment_value_names_the_environment_as_its_layer() {
    let err = load_from_env(
        &CliOverrides::default(),
        env(&[("RHIZO_EDGE__CONTROL__TICK_INTERVAL_SECONDS", "thirty")]),
    )
    .unwrap_err();
    assert_eq!(err.key(), "control.tick_interval_seconds");
    assert!(
        err.to_string().contains("environment"),
        "the message must say which layer supplied it: {err}"
    );
}

#[test]
fn an_unknown_key_is_rejected_rather_than_silently_ignored() {
    // A typo like `tick_interval_second` would otherwise leave the default in
    // place while the operator believed they had changed it.
    let f = TempToml::new(
        r#"
[control]
tick_interval_second = 15
command_ttl_seconds = 120
"#,
    );
    let err = load_from_env(&f.cli(), env(&[])).unwrap_err();
    assert!(
        err.to_string().contains("tick_interval_second"),
        "the unknown key must be named: {err}"
    );
}

#[test]
fn a_zero_tick_interval_is_rejected() {
    assert_rejected(
        &CliOverrides::default(),
        &[("RHIZO_EDGE__CONTROL__TICK_INTERVAL_SECONDS", "0")],
        "control.tick_interval_seconds",
    );
}

#[test]
fn a_zero_command_ttl_is_rejected() {
    assert_rejected(
        &CliOverrides::default(),
        &[("RHIZO_EDGE__CONTROL__COMMAND_TTL_SECONDS", "0")],
        "control.command_ttl_seconds",
    );
}

#[test]
fn a_command_ttl_shorter_than_the_tick_interval_is_rejected() {
    assert_rejected(
        &CliOverrides::default(),
        &[
            ("RHIZO_EDGE__CONTROL__TICK_INTERVAL_SECONDS", "60"),
            ("RHIZO_EDGE__CONTROL__COMMAND_TTL_SECONDS", "30"),
        ],
        "control.command_ttl_seconds",
    );
}

#[test]
fn an_invalid_edge_id_is_rejected() {
    for bad in [
        "", "ab", "Home-01", "home_01", "home/01", "home+01", "home#01",
    ] {
        assert_rejected(
            &CliOverrides::default(),
            &[("RHIZO_EDGE__EDGE_ID", bad)],
            "edge_id",
        );
    }
}

#[test]
fn a_broker_url_without_a_scheme_is_rejected() {
    assert_rejected(
        &CliOverrides::default(),
        &[("RHIZO_EDGE__MQTT__BROKER_URL", "localhost:1883")],
        "mqtt.broker_url",
    );
}

#[test]
fn a_broker_url_with_the_wrong_scheme_is_rejected() {
    assert_rejected(
        &CliOverrides::default(),
        &[("RHIZO_EDGE__MQTT__BROKER_URL", "http://localhost:1883")],
        "mqtt.broker_url",
    );
}

#[test]
fn a_broker_url_with_a_nonsense_port_is_rejected() {
    assert_rejected(
        &CliOverrides::default(),
        &[(
            "RHIZO_EDGE__MQTT__BROKER_URL",
            "mqtt://localhost:not-a-port",
        )],
        "mqtt.broker_url",
    );
}

#[test]
fn a_broker_url_without_a_host_is_rejected() {
    assert_rejected(
        &CliOverrides::default(),
        &[("RHIZO_EDGE__MQTT__BROKER_URL", "mqtt://:1883")],
        "mqtt.broker_url",
    );
}

#[test]
fn an_mqtts_broker_url_is_accepted_even_though_tls_is_a_later_milestone() {
    // Refusing the scheme outright would make M13's TLS work a configuration
    // change here as well, for no gain today.
    let c = load_from_env(
        &CliOverrides::default(),
        env(&[("RHIZO_EDGE__MQTT__BROKER_URL", "mqtts://broker.local:8883")]),
    )
    .unwrap()
    .config;
    assert_eq!(c.mqtt.broker_url, "mqtts://broker.local:8883");
}

#[test]
fn an_empty_mqtt_username_is_rejected() {
    assert_rejected(
        &CliOverrides::default(),
        &[("RHIZO_EDGE__MQTT__USERNAME", "")],
        "mqtt.username",
    );
}

#[test]
fn an_empty_mqtt_client_id_is_rejected() {
    assert_rejected(
        &CliOverrides::default(),
        &[("RHIZO_EDGE__MQTT__CLIENT_ID", "")],
        "mqtt.client_id",
    );
}

#[test]
fn an_unparseable_api_bind_is_rejected() {
    let err = load_from_env(
        &CliOverrides::default(),
        env(&[("RHIZO_EDGE__API__BIND", "not-an-address")]),
    )
    .unwrap_err();
    assert_eq!(err.key(), "api.bind");
}

#[test]
fn an_invalid_log_level_is_rejected() {
    assert_rejected(
        &CliOverrides::default(),
        &[("RHIZO_EDGE__LOG__LEVEL", "info=info=info")],
        "log.level",
    );
}

#[test]
fn an_invalid_log_format_is_rejected_and_lists_the_accepted_values() {
    let err = load_from_env(
        &CliOverrides::default(),
        env(&[("RHIZO_EDGE__LOG__FORMAT", "yaml")]),
    )
    .unwrap_err();
    assert_eq!(err.key(), "log.format");
    assert!(err.to_string().contains("json"), "{err}");
    assert!(err.to_string().contains("pretty"), "{err}");
}

#[test]
fn a_bad_cloud_base_url_is_rejected_only_when_the_cloud_is_enabled() {
    // Disabled: not validated. Validating a URL nothing will ever call would
    // block a working, cloud-less edge over a field it does not use.
    let ok = load_from_env(
        &CliOverrides::default(),
        env(&[("RHIZO_EDGE__CLOUD__BASE_URL", "nonsense")]),
    );
    assert!(ok.is_ok(), "a disabled cloud must not gate startup");

    assert_rejected(
        &CliOverrides::default(),
        &[
            ("RHIZO_EDGE__CLOUD__ENABLED", "true"),
            ("RHIZO_EDGE__CLOUD__BASE_URL", "nonsense"),
        ],
        "cloud.base_url",
    );
}

#[test]
fn a_valid_cloud_configuration_is_accepted() {
    let c = load_from_env(
        &CliOverrides::default(),
        env(&[
            ("RHIZO_EDGE__CLOUD__ENABLED", "true"),
            ("RHIZO_EDGE__CLOUD__BASE_URL", "https://cloud.example:8081"),
        ]),
    )
    .unwrap()
    .config;
    assert!(c.cloud.enabled);
    assert_eq!(c.cloud.base_url, "https://cloud.example:8081");
}

// --------------------------------------------------------------- log format

#[test]
fn the_parsed_log_format_matches_the_configured_string() {
    let c = load_from_env(
        &CliOverrides::default(),
        env(&[("RHIZO_EDGE__LOG__FORMAT", "pretty")]),
    )
    .unwrap()
    .config;
    assert_eq!(
        c.log.parsed_format().unwrap(),
        rhizo_telemetry::LogFormat::Pretty
    );
}

#[test]
fn the_default_log_format_is_json() {
    assert_eq!(
        defaults().log.parsed_format().unwrap(),
        rhizo_telemetry::LogFormat::Json
    );
}
