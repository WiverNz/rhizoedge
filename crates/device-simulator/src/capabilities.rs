//! What this device can sense and actuate.
//!
//! [ADR-016](../../../../docs/adr/016-plant-binding-and-policy-model.md) forbids
//! the edge from assuming what a device can do: `device == pump controller` is
//! exactly the assumption that makes monitoring-only plants second-class, so
//! capabilities are **declared**, not inferred.
//!
//! # One source, two consumers
//!
//! The same table drives the declaration in `device.status` *and* the sampling
//! loop. A device that declared `illuminance` and never sent it would be a bug
//! the conformance test should catch, and the cheapest way to make that
//! impossible is to have one definition rather than two that must agree.

use rhizo_mqtt_contract::payload::{
    ActuatorCapability, DeviceCapabilities, MeasurementKind, MeasurementPoint, SensorCapability,
    SensorId,
};

use crate::cli::{Cli, SensorGroup};

/// One simulated sensor.
#[derive(Clone, Debug, PartialEq)]
pub struct SensorSpec {
    /// Stable id, unchanged across reboots.
    pub sensor_id: SensorId,
    /// Default measurement point for its samples.
    pub point: MeasurementPoint,
    /// The hardware group that brings it into existence.
    pub group: SensorGroup,
    /// The kinds it produces, in publication order.
    pub kinds: Vec<MeasurementKind>,
    /// Calibration state. `None` means calibration does not apply.
    pub calibrated: Option<bool>,
}

/// The device's declared capabilities.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Capabilities {
    sensors: Vec<SensorSpec>,
    actuators: Vec<ActuatorCapability>,
}

impl Capabilities {
    /// Derives capabilities from `--sensors` and `--actuators`.
    ///
    /// The soil probe becomes **two** declared sensors. One assembly, but its
    /// moisture and temperature channels are factory-calibrated while its
    /// conductivity channel is not, and `calibrated` is per sensor. Declaring
    /// one sensor would force a single answer to a question with two: either
    /// the EC samples would claim a calibration they do not have, or the
    /// moisture samples would disclaim one they do.
    #[must_use]
    pub fn from_cli(cli: &Cli) -> Self {
        let mut sensors = Vec::new();
        for group in cli.sensors.groups() {
            match group {
                SensorGroup::Soil => {
                    sensors.push(SensorSpec {
                        sensor_id: local_id("soil-0"),
                        point: point("default"),
                        group: SensorGroup::Soil,
                        kinds: vec![
                            MeasurementKind::SoilMoisture,
                            MeasurementKind::SoilTemperature,
                        ],
                        calibrated: Some(true),
                    });
                    sensors.push(SensorSpec {
                        sensor_id: local_id("ec-0"),
                        point: point("default"),
                        group: SensorGroup::Soil,
                        kinds: vec![MeasurementKind::SoilEc],
                        // A cheap conductivity probe is not calibrated, and
                        // saying so makes its samples advisory at the edge
                        // rather than silently usable for control.
                        calibrated: Some(false),
                    });
                }
                SensorGroup::Weight => sensors.push(SensorSpec {
                    sensor_id: local_id("weight-0"),
                    point: point("default"),
                    group: SensorGroup::Weight,
                    kinds: vec![MeasurementKind::PotWeight],
                    calibrated: Some(true),
                }),
                SensorGroup::Tank => sensors.push(SensorSpec {
                    sensor_id: local_id("tank-0"),
                    point: point("reservoir"),
                    group: SensorGroup::Tank,
                    kinds: vec![MeasurementKind::TankLevel],
                    calibrated: None,
                }),
                SensorGroup::Leak => sensors.push(SensorSpec {
                    sensor_id: local_id("leak-0"),
                    point: point("tray"),
                    group: SensorGroup::Leak,
                    kinds: vec![MeasurementKind::LeakState],
                    calibrated: None,
                }),
            }
        }
        let actuators = cli
            .actuators
            .specs()
            .iter()
            .map(|spec| ActuatorCapability {
                actuator_id: spec.actuator_id.clone(),
                kind: spec.kind,
                present: true,
                healthy: true,
            })
            .collect();
        Self { sensors, actuators }
    }

    /// The declared sensors, in publication order.
    #[must_use]
    pub fn sensors(&self) -> &[SensorSpec] {
        &self.sensors
    }

    /// The declared actuators. An empty list is a normal monitoring device.
    #[must_use]
    pub fn actuators(&self) -> &[ActuatorCapability] {
        &self.actuators
    }

    /// The first declared actuator, which is the one a water command drives.
    #[must_use]
    pub fn primary_actuator(&self) -> Option<&ActuatorCapability> {
        self.actuators.first()
    }

    /// Whether an actuator id was declared.
    ///
    /// A policy naming an undeclared actuator is rejected (SAFETY-018), and
    /// this is the check that rejects it.
    #[must_use]
    pub fn declares_actuator(&self, actuator_id: &SensorId) -> bool {
        self.actuators
            .iter()
            .any(|a| a.actuator_id.as_str() == actuator_id.as_str())
    }

    /// Whether a kind is produced at a point by some declared sensor.
    #[must_use]
    pub fn produces(&self, kind: &MeasurementKind, point: &MeasurementPoint) -> bool {
        self.sensors
            .iter()
            .any(|s| s.point.as_str() == point.as_str() && s.kinds.iter().any(|k| k == kind))
    }

    /// Every kind this device samples, in publication order.
    #[must_use]
    pub fn sampled_kinds(&self) -> Vec<MeasurementKind> {
        self.sensors
            .iter()
            .flat_map(|s| s.kinds.iter().cloned())
            .collect()
    }

    /// Builds the wire declaration.
    ///
    /// `healthy` reports per-sensor health so an injected sensor fault is
    /// visible in status as well as in the sample's `quality`.
    pub fn declaration(
        &self,
        mut healthy: impl FnMut(&SensorSpec) -> (bool, u32),
    ) -> DeviceCapabilities {
        DeviceCapabilities {
            sensors: self
                .sensors
                .iter()
                .map(|spec| {
                    let (is_healthy, errors) = healthy(spec);
                    SensorCapability {
                        sensor_id: spec.sensor_id.clone(),
                        point: spec.point.clone(),
                        kinds: spec.kinds.clone(),
                        present: true,
                        healthy: is_healthy,
                        errors,
                        calibrated: spec.calibrated,
                    }
                })
                .collect(),
            actuators: self.actuators.clone(),
        }
    }
}

/// Parses a compile-time-known local identifier.
///
/// `expect` is the documented exception to the workspace lint (root
/// `Cargo.toml`): the invariant genuinely cannot be violated, because every
/// caller passes a literal that is valid by inspection, and the message states
/// why. The alternative — dropping a sensor whose id failed to parse — would
/// silently produce a device that declares less than it samples, which is the
/// one thing this module exists to prevent.
#[allow(clippy::expect_used)]
///
/// The literals here are all valid by inspection, so a failure would be a
/// programming error rather than a runtime condition — but the crate denies
/// `unwrap`, and falling back to a wrong id silently would be worse than the
/// panic. `expect` with a reason is the documented exception.
fn local_id(value: &str) -> SensorId {
    SensorId::parse(value).expect("built-in sensor ids are valid local identifiers")
}

#[allow(clippy::expect_used)]
fn point(value: &str) -> MeasurementPoint {
    MeasurementPoint::parse(value).expect("built-in points are valid local identifiers")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::cli;

    #[test]
    fn the_default_device_declares_every_group_it_samples() {
        let capabilities = Capabilities::from_cli(&cli(&[]));
        let ids: Vec<_> = capabilities
            .sensors()
            .iter()
            .map(|s| s.sensor_id.as_str())
            .collect();
        assert_eq!(ids, ["soil-0", "ec-0", "weight-0", "tank-0", "leak-0"]);
        assert_eq!(
            capabilities.sampled_kinds(),
            vec![
                MeasurementKind::SoilMoisture,
                MeasurementKind::SoilTemperature,
                MeasurementKind::SoilEc,
                MeasurementKind::PotWeight,
                MeasurementKind::TankLevel,
                MeasurementKind::LeakState,
            ]
        );
    }

    #[test]
    fn a_disabled_group_declares_nothing() {
        let capabilities = Capabilities::from_cli(&cli(&["--sensors", "soil"]));
        assert_eq!(
            capabilities.sampled_kinds(),
            vec![
                MeasurementKind::SoilMoisture,
                MeasurementKind::SoilTemperature,
                MeasurementKind::SoilEc,
            ]
        );
        assert!(!capabilities.produces(
            &MeasurementKind::TankLevel,
            &MeasurementPoint::parse("reservoir").unwrap()
        ));
    }

    #[test]
    fn a_device_with_no_sensors_at_all_is_representable() {
        let capabilities = Capabilities::from_cli(&cli(&["--sensors", ""]));
        assert!(capabilities.sensors().is_empty());
        assert!(capabilities.sampled_kinds().is_empty());
    }

    #[test]
    fn a_monitoring_only_device_declares_an_empty_actuator_list() {
        let capabilities = Capabilities::from_cli(&cli(&["--actuators", ""]));
        assert!(capabilities.actuators().is_empty());
        assert!(capabilities.primary_actuator().is_none());
        assert!(!capabilities.declares_actuator(&SensorId::parse("pump-0").unwrap()));
        // ...and it still senses everything.
        assert_eq!(capabilities.sensors().len(), 5);
    }

    #[test]
    fn an_undeclared_actuator_is_not_recognised() {
        let capabilities = Capabilities::from_cli(&cli(&["--actuators", "pump-0"]));
        assert!(capabilities.declares_actuator(&SensorId::parse("pump-0").unwrap()));
        assert!(!capabilities.declares_actuator(&SensorId::parse("pump-9").unwrap()));
    }

    #[test]
    fn sensor_ids_are_stable_across_construction() {
        let first = Capabilities::from_cli(&cli(&[]));
        let second = Capabilities::from_cli(&cli(&[]));
        assert_eq!(first, second, "ids must survive a restart unchanged");
    }

    #[test]
    fn the_conductivity_channel_declares_that_it_is_uncalibrated() {
        let capabilities = Capabilities::from_cli(&cli(&["--sensors", "soil"]));
        let ec = capabilities
            .sensors()
            .iter()
            .find(|s| s.kinds.contains(&MeasurementKind::SoilEc))
            .unwrap();
        assert_eq!(ec.calibrated, Some(false));
        let soil = capabilities
            .sensors()
            .iter()
            .find(|s| s.kinds.contains(&MeasurementKind::SoilMoisture))
            .unwrap();
        assert_eq!(soil.calibrated, Some(true));
        assert_ne!(ec.sensor_id.as_str(), soil.sensor_id.as_str());
    }

    #[test]
    fn the_declaration_carries_health_from_the_caller() {
        let capabilities = Capabilities::from_cli(&cli(&["--sensors", "tank"]));
        let declaration = capabilities.declaration(|_| (false, 3));
        assert_eq!(declaration.sensors.len(), 1);
        assert!(!declaration.sensors[0].healthy);
        assert_eq!(declaration.sensors[0].errors, 3);
        assert!(declaration.sensors[0].present);
    }

    #[test]
    fn the_declaration_matches_what_is_sampled_exactly() {
        for flags in [
            vec![],
            vec!["--sensors", "soil"],
            vec!["--sensors", "tank,leak"],
            vec!["--sensors", "weight"],
        ] {
            let capabilities = Capabilities::from_cli(&cli(&flags));
            let declared: Vec<MeasurementKind> = capabilities
                .declaration(|_| (true, 0))
                .sensors
                .into_iter()
                .flat_map(|s| s.kinds)
                .collect();
            assert_eq!(
                declared,
                capabilities.sampled_kinds(),
                "a device must not declare what it does not publish: {flags:?}"
            );
        }
    }
}
