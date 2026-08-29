//! Retained device configuration.
//!
//! Protocol §5.7. The edge owns `config_version`; the device validates, applies,
//! and echoes it back as `applied_config_version`, which is how configuration
//! drift is detected.
//!
//! # Two rules that fail silently when omitted
//!
//! **A version at or below the applied one is ignored.** Without this a
//! rollback that republishes an old retained config silently regresses the
//! device, and nothing in the system reports it.
//!
//! **An invalid config is rejected, never clamped, and the previous one stays
//! in force.** Half-applying a configuration is how a device ends up running a
//! new interval with an old pump calibration.

use rhizo_mqtt_contract::payload::{ConfigError, DeviceConfig, PumpConfig, TankConfig};

use crate::cli::Cli;

/// The configuration the device is actually running.
///
/// Seeded from the command line — a device has working defaults before the edge
/// has ever spoken to it — and replaced wholesale by an accepted `device.config`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectiveConfig {
    /// The applied version, or `None` before any config has been accepted.
    pub applied_version: Option<u32>,
    /// Sampling interval, seconds of virtual time.
    pub telemetry_interval_seconds: u32,
    /// Pump tuning. Never a safety limit.
    pub pump: PumpConfig,
    /// Tank threshold tuning.
    pub tank: TankConfig,
}

impl EffectiveConfig {
    /// The device's pre-configuration defaults, from the command line.
    #[must_use]
    pub fn from_cli(cli: &Cli) -> Self {
        Self {
            applied_version: None,
            telemetry_interval_seconds: cli.telemetry_interval,
            pump: PumpConfig {
                ml_per_second: cli.ml_per_second,
                // A device with no declared actuator has no pump to enable.
                enabled: !cli.actuators.specs().is_empty(),
            },
            tank: TankConfig { min_percent: 15.0 },
        }
    }

    /// The status heartbeat period: five sampling intervals (protocol §5.5).
    #[must_use]
    pub const fn heartbeat_interval_ms(&self) -> u64 {
        (self.telemetry_interval_seconds as u64) * 5 * 1000
    }

    /// The sampling period in milliseconds.
    #[must_use]
    pub const fn telemetry_interval_ms(&self) -> u64 {
        (self.telemetry_interval_seconds as u64) * 1000
    }

    /// Considers an incoming configuration.
    ///
    /// Returns what happened, so the caller can decide whether to republish
    /// status and can log the reason a config was refused.
    pub fn consider(&mut self, incoming: &DeviceConfig) -> ConfigOutcome {
        if let Some(applied) = self.applied_version
            && incoming.config_version <= applied
        {
            return ConfigOutcome::IgnoredVersion {
                offered: incoming.config_version,
                applied,
            };
        }
        // Validation is the contract crate's, so the device and the firmware
        // cannot disagree about what a valid configuration is.
        if let Err(error) = incoming.validate() {
            return ConfigOutcome::Rejected { error };
        }
        // `sensors` is deliberately not applied. Protocol §5.7's normative field
        // table covers `telemetry_interval_seconds`, `pump`, and `tank` only;
        // the `sensors` block describes hardware presence, which a message
        // cannot change, and M2-015 requires the declared capabilities to match
        // what is actually sampled. Decoding it and ignoring it is the forward-
        // compatible behaviour §9 asks for.
        self.applied_version = Some(incoming.config_version);
        self.telemetry_interval_seconds = incoming.telemetry_interval_seconds;
        self.pump = incoming.pump;
        self.tank = incoming.tank;
        ConfigOutcome::Applied {
            version: incoming.config_version,
        }
    }
}

/// What happened to an incoming `device.config`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigOutcome {
    /// Validated and applied; the device now reports this version.
    Applied {
        /// The new applied version.
        version: u32,
    },
    /// Rejected; the previously applied configuration is untouched.
    Rejected {
        /// Which bound was violated.
        error: ConfigError,
    },
    /// At or below the applied version, so ignored without validation.
    IgnoredVersion {
        /// The version offered.
        offered: u32,
        /// The version already applied.
        applied: u32,
    },
}

impl ConfigOutcome {
    /// Whether the device's behaviour changed, and therefore whether status
    /// must be republished.
    #[must_use]
    pub const fn changed(self) -> bool {
        matches!(self, Self::Applied { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::cli;

    fn config(version: u32) -> DeviceConfig {
        DeviceConfig {
            config_version: version,
            telemetry_interval_seconds: 300,
            pump: PumpConfig {
                ml_per_second: 8.2,
                enabled: true,
            },
            tank: TankConfig { min_percent: 15.0 },
            sensors: Default::default(),
            power: None,
        }
    }

    #[test]
    fn a_device_has_working_defaults_before_the_edge_speaks_to_it() {
        let effective = EffectiveConfig::from_cli(&cli(&["--telemetry-interval", "60"]));
        assert_eq!(effective.applied_version, None);
        assert_eq!(effective.telemetry_interval_seconds, 60);
        assert_eq!(effective.heartbeat_interval_ms(), 300_000);
    }

    #[test]
    fn a_monitoring_only_device_has_no_pump_to_enable() {
        let effective = EffectiveConfig::from_cli(&cli(&["--actuators", ""]));
        assert!(
            !effective.pump.enabled,
            "a plant with no actuator is a normal monitoring plant (SAFETY-018)"
        );
    }

    #[test]
    fn a_valid_config_is_applied_and_echoed() {
        let mut effective = EffectiveConfig::from_cli(&cli(&[]));
        let mut incoming = config(7);
        incoming.telemetry_interval_seconds = 120;
        assert_eq!(
            effective.consider(&incoming),
            ConfigOutcome::Applied { version: 7 }
        );
        assert_eq!(effective.applied_version, Some(7));
        assert_eq!(effective.telemetry_interval_seconds, 120);
        assert_eq!(effective.heartbeat_interval_ms(), 600_000);
    }

    #[test]
    fn an_invalid_config_is_rejected_and_the_previous_one_survives_intact() {
        let mut effective = EffectiveConfig::from_cli(&cli(&[]));
        effective.consider(&config(7));
        let before = effective;

        let mut bad = config(8);
        bad.telemetry_interval_seconds = 9;
        assert_eq!(
            effective.consider(&bad),
            ConfigOutcome::Rejected {
                error: ConfigError::TelemetryInterval
            }
        );
        assert_eq!(effective, before, "nothing may be half-applied");

        let mut bad = config(9);
        bad.pump.ml_per_second = 0.0;
        assert!(matches!(
            effective.consider(&bad),
            ConfigOutcome::Rejected {
                error: ConfigError::PumpRate
            }
        ));
        let mut bad = config(10);
        bad.tank.min_percent = 101.0;
        assert!(matches!(
            effective.consider(&bad),
            ConfigOutcome::Rejected {
                error: ConfigError::TankMinimum
            }
        ));
        assert_eq!(effective, before);
    }

    #[test]
    fn a_lower_or_equal_version_is_ignored_without_being_validated() {
        let mut effective = EffectiveConfig::from_cli(&cli(&[]));
        effective.consider(&config(7));
        for offered in [1, 6, 7] {
            let mut incoming = config(offered);
            incoming.telemetry_interval_seconds = 11;
            assert_eq!(
                effective.consider(&incoming),
                ConfigOutcome::IgnoredVersion {
                    offered,
                    applied: 7
                },
                "a rollback republishing an old retained config must not regress the device"
            );
            assert_eq!(effective.telemetry_interval_seconds, 300);
        }
        let mut incoming = config(8);
        incoming.telemetry_interval_seconds = 11;
        assert!(effective.consider(&incoming).changed());
    }

    #[test]
    fn unrecognised_fields_including_smuggled_limits_are_ignored() {
        let json = r#"{
            "config_version": 3,
            "telemetry_interval_seconds": 60,
            "pump": {"ml_per_second": 8.2, "enabled": true},
            "tank": {"min_percent": 15.0},
            "sensors": {"soil": true},
            "ntp_server": "pool.ntp.org",
            "max_ml_per_run": 9999.0,
            "max_daily_ml": 100000.0
        }"#;
        let incoming: DeviceConfig = serde_json::from_str(json).unwrap();
        let mut effective = EffectiveConfig::from_cli(&cli(&[]));
        assert!(effective.consider(&incoming).changed());
        assert_eq!(effective.telemetry_interval_seconds, 60);
        // The smuggled fields do not exist on the type at all, so they cannot
        // have been stored anywhere. The limits the device reports come from
        // the contract's constants (SAFETY-007), asserted in `device`.
    }

    #[test]
    fn only_an_applied_config_counts_as_a_change() {
        assert!(ConfigOutcome::Applied { version: 1 }.changed());
        assert!(
            !ConfigOutcome::Rejected {
                error: ConfigError::PumpRate
            }
            .changed()
        );
        assert!(
            !ConfigOutcome::IgnoredVersion {
                offered: 1,
                applied: 2
            }
            .changed()
        );
    }
}
