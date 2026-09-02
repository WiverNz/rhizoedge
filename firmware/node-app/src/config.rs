//! Device configuration handling (M9-012, protocol §5.7).
//!
//! Three rules, each of which is invisible when it is missing:
//!
//! * **`config_version <= applied` is ignored.** Its absence only shows up
//!   after a rollback republishes an old retained config, which is exactly when
//!   nobody is looking for it.
//! * **Invalid configuration is rejected and the previous retained**, never
//!   partially applied.
//! * **Unrecognised fields are ignored**, which is what makes adding a config
//!   field non-breaking across mixed firmware versions — and also means an
//!   attempt to smuggle a safety limit through the config topic has no effect.
//!   The hard limits are compile-time constants in the shared contract crate
//!   and no message can reach them (ADR-011, SAFETY-007).

use rhizo_mqtt_contract::payload::{ConfigError, DeviceConfig, PowerMode};

use crate::persist::PersistedState;

/// What an inbound configuration did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigOutcome {
    /// Applied and persisted; status should echo the new version.
    Applied {
        /// The version now in force.
        config_version: u32,
    },
    /// Its version is not newer than the applied one.
    IgnoredNotNewer {
        /// The version offered.
        offered: u32,
    },
    /// It failed validation; the previous configuration is retained.
    Rejected(ConfigError),
}

/// Defaults used only for values a configuration has never supplied.
///
/// These are *tuning* defaults, never safety limits: none of them can permit a
/// dose, and every one of them is overridden by the first valid configuration.
pub mod defaults {
    /// Sampling cadence when no configuration has arrived.
    pub const TELEMETRY_INTERVAL_SECONDS: u32 = 300;
    /// Wake cadence for a battery device with no interval configured.
    pub const WAKE_INTERVAL_SECONDS: u32 = 900;
    /// Idle awake budget when none is configured.
    pub const AWAKE_BUDGET_SECONDS: u32 = 60;
    /// Peripheral warm-up when none is configured.
    ///
    /// **Not a value for any particular sensor part.** M10-011 measures the
    /// real figure for the probe that is actually fitted; until then a
    /// configuration carries a conservative value marked as unmeasured, and
    /// this is only the fallback for a device that has never been configured at
    /// all (M9-020 non-goals, F-090-56).
    pub const SENSOR_WARMUP_MS: u32 = 1_000;
}

/// Applies a retained `device.config`.
pub fn apply(state: &mut PersistedState, config: &DeviceConfig) -> ConfigOutcome {
    if let Some(applied) = state.config_version
        && config.config_version <= applied
    {
        return ConfigOutcome::IgnoredNotNewer {
            offered: config.config_version,
        };
    }
    if let Err(error) = config.validate() {
        return ConfigOutcome::Rejected(error);
    }
    state.config = Some(*config);
    state.config_version = Some(config.config_version);
    ConfigOutcome::Applied {
        config_version: config.config_version,
    }
}

/// The effective sampling cadence.
#[must_use]
pub fn telemetry_interval_seconds(state: &PersistedState) -> u32 {
    state
        .config
        .map_or(defaults::TELEMETRY_INTERVAL_SECONDS, |config| {
            config.telemetry_interval_seconds
        })
}

/// The effective power mode, resolved conservatively.
///
/// An absent `power` block and an unrecognised mode both yield
/// [`PowerMode::AlwaysOn`] (F-090-50). The resolution itself lives in the
/// shared contract's `PowerMode::effective`, so there is one copy of
/// "uncertainty must not choose the branch that makes a device unreachable".
#[must_use]
pub fn power_mode(state: &PersistedState) -> PowerMode {
    state
        .config
        .and_then(|config| config.power)
        .map_or(PowerMode::AlwaysOn, |power| power.effective_mode())
}

/// The effective wake interval for a battery device.
#[must_use]
pub fn wake_interval_seconds(state: &PersistedState) -> u32 {
    state
        .config
        .and_then(|config| config.power)
        .and_then(|power| power.wake_interval_seconds)
        .unwrap_or(defaults::WAKE_INTERVAL_SECONDS)
}

/// The configured peripheral warm-up.
#[must_use]
pub fn sensor_warmup_ms(state: &PersistedState) -> u32 {
    state
        .config
        .and_then(|config| config.power)
        .and_then(|power| power.sensor_warmup_ms)
        .unwrap_or(defaults::SENSOR_WARMUP_MS)
}

/// The configured idle awake budget.
#[must_use]
pub fn awake_budget_seconds(state: &PersistedState) -> u32 {
    state
        .config
        .and_then(|config| config.power)
        .and_then(|power| power.awake_budget_seconds)
        .unwrap_or(defaults::AWAKE_BUDGET_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhizo_mqtt_contract::payload::{PowerConfig, PumpConfig, SensorConfig, TankConfig};

    fn config(version: u32) -> DeviceConfig {
        DeviceConfig {
            config_version: version,
            telemetry_interval_seconds: 300,
            pump: PumpConfig {
                ml_per_second: 8.0,
                enabled: true,
            },
            tank: TankConfig { min_percent: 15.0 },
            sensors: SensorConfig {
                soil: true,
                weight: true,
                tank: true,
                leak: true,
            },
            power: None,
        }
    }

    #[test]
    fn a_valid_config_is_applied_and_persisted() {
        let mut state = PersistedState::default();
        assert_eq!(
            apply(&mut state, &config(4)),
            ConfigOutcome::Applied { config_version: 4 }
        );
        assert_eq!(state.config_version, Some(4));
        let encoded = serde_json::to_vec(&state).expect("encodes");
        let restored: PersistedState = serde_json::from_slice(&encoded).expect("decodes");
        assert_eq!(restored.config_version, Some(4));
    }

    #[test]
    fn a_lower_or_equal_version_is_ignored() {
        let mut state = PersistedState::default();
        apply(&mut state, &config(4));
        assert_eq!(
            apply(&mut state, &config(3)),
            ConfigOutcome::IgnoredNotNewer { offered: 3 }
        );
        assert_eq!(
            apply(&mut state, &config(4)),
            ConfigOutcome::IgnoredNotNewer { offered: 4 }
        );
        assert_eq!(state.config_version, Some(4));
    }

    #[test]
    fn an_invalid_config_is_rejected_and_the_previous_retained() {
        let mut state = PersistedState::default();
        apply(&mut state, &config(1));
        let mut bad = config(2);
        bad.pump.ml_per_second = 0.0;
        assert_eq!(
            apply(&mut state, &bad),
            ConfigOutcome::Rejected(ConfigError::PumpRate)
        );
        assert_eq!(state.config_version, Some(1));
        assert_eq!(
            state.config.map(|c| c.pump.ml_per_second),
            Some(8.0),
            "the previous pump calibration survives"
        );
    }

    /// SAFETY-007: a configuration cannot reach the hard limits, because the
    /// hard limits are not in the configuration type at all. This test exists
    /// so that a future field named `max_ml_per_run` would have to break it.
    #[test]
    fn safety_007_no_configuration_field_can_change_a_reported_limit() {
        let mut state = PersistedState::default();
        let raw = serde_json::json!({
            "config_version": 9,
            "telemetry_interval_seconds": 300,
            "pump": { "ml_per_second": 8.0, "enabled": true },
            "tank": { "min_percent": 15.0 },
            "max_ml_per_run": 5000.0,
            "firmware_max_daily_ml": 99999.0
        });
        let decoded: DeviceConfig = serde_json::from_value(raw).expect("unknown fields ignored");
        assert_eq!(
            apply(&mut state, &decoded),
            ConfigOutcome::Applied { config_version: 9 }
        );
        assert_eq!(
            rhizo_mqtt_contract::safety::FIRMWARE_MAX_ML_PER_RUN,
            80.0,
            "the limit is a compile-time constant and no message reaches it"
        );
        assert_eq!(rhizo_mqtt_contract::safety::FIRMWARE_MAX_DAILY_ML, 500.0);
    }

    #[test]
    fn an_absent_or_unrecognised_power_mode_yields_always_on() {
        let mut state = PersistedState::default();
        apply(&mut state, &config(1));
        assert_eq!(power_mode(&state), PowerMode::AlwaysOn);

        let mut with_unknown = config(2);
        with_unknown.power = Some(PowerConfig {
            mode: PowerMode::Unknown,
            ..PowerConfig::default()
        });
        apply(&mut state, &with_unknown);
        assert_eq!(power_mode(&state), PowerMode::AlwaysOn);

        let mut battery = config(3);
        battery.power = Some(PowerConfig {
            mode: PowerMode::Battery,
            wake_interval_seconds: Some(900),
            sensor_warmup_ms: Some(2_500),
            awake_budget_seconds: Some(45),
        });
        apply(&mut state, &battery);
        assert_eq!(power_mode(&state), PowerMode::Battery);
        assert_eq!(wake_interval_seconds(&state), 900);
        assert_eq!(sensor_warmup_ms(&state), 2_500);
        assert_eq!(awake_budget_seconds(&state), 45);
    }
}
