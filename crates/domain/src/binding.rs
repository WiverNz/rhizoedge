//! Binding validation (M5-013, [ADR-016](../../../docs/adr/016-plant-binding-and-policy-model.md)).
//!
//! A rule names a **kind and a role**; a binding maps that to hardware.
//! Replacing a failed probe is then a binding edit rather than a data migration,
//! and the plant's policies and history are untouched by it.
//!
//! # The three roles are not interchangeable
//!
//! - `control` drives the decision. At most one per plant, and it must be a
//!   recognised **scalar** kind, because "below target for 30 minutes" has no
//!   meaning for a boolean.
//! - `required` must be healthy for actuation to be safe.
//! - `advisory` is recorded and may alert, but never gates the pump.
//!
//! Marking a leak sensor `advisory` would silently remove its veto, which is why
//! [`validate_sensor_binding`] refuses it for any plant that has an actuator.
//!
//! # Zero actuator bindings is the common case
//!
//! Not a degraded one (SAFETY-018). A monitoring-only plant is creatable,
//! viewable, and alertable; it simply has no actuation route.
use crate::plant::{ActuatorBinding, BindingRole, SensorBinding};
use rhizo_mqtt_contract::payload::{ActuatorKind, MeasurementKind};

/// One sensor capability a device declared (M4-011).
#[derive(Clone, Debug, PartialEq)]
pub struct DeclaredSensor {
    /// Owning device.
    pub device_id: String,
    /// Capability id.
    pub sensor_id: String,
    /// Measurement point.
    pub point: String,
    /// Kinds this sensor produces.
    pub kinds: Vec<MeasurementKind>,
}

/// One actuator capability a device declared.
#[derive(Clone, Debug, PartialEq)]
pub struct DeclaredActuator {
    /// Owning device.
    pub device_id: String,
    /// Capability id.
    pub actuator_id: String,
    /// Actuator kind.
    pub kind: ActuatorKind,
}

/// What the fleet has actually declared. A binding may name nothing else.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeclaredCapabilities {
    /// Declared sensors.
    pub sensors: Vec<DeclaredSensor>,
    /// Declared actuators.
    pub actuators: Vec<DeclaredActuator>,
}

/// A refused binding, one variant per rule.
#[derive(Clone, Debug, PartialEq)]
pub enum BindingError {
    /// No such sensor capability on that device.
    UndeclaredSensor {
        /// Device named by the binding.
        device_id: String,
        /// Sensor named by the binding.
        sensor_id: String,
    },
    /// The sensor exists but does not produce that kind at that point.
    SensorDoesNotProduceKind {
        /// Sensor named by the binding.
        sensor_id: String,
        /// Kind named by the binding.
        kind: String,
    },
    /// No such actuator capability on that device.
    UndeclaredActuator {
        /// Device named by the binding.
        device_id: String,
        /// Actuator named by the binding.
        actuator_id: String,
    },
    /// V1 actuates irrigation pumps and nothing else.
    UnsupportedActuatorKind {
        /// Kind named by the binding.
        kind: String,
    },
    /// The plant already has a `control` binding.
    DuplicateControlBinding,
    /// A control binding must be a recognised scalar kind.
    ControlKindNotEligible {
        /// Kind named by the binding.
        kind: String,
    },
    /// A leak or tank binding on a plant with an actuator must be `required`.
    SafetyRoleMustBeRequired {
        /// Kind named by the binding.
        kind: String,
    },
    /// Removing the last `control` binding while automation is enabled.
    LastControlBindingWhileAutomationEnabled,
}

impl BindingError {
    /// The stable API error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UndeclaredSensor { .. } => "undeclared_sensor",
            Self::SensorDoesNotProduceKind { .. } => "sensor_does_not_produce_kind",
            Self::UndeclaredActuator { .. } => "undeclared_actuator",
            Self::UnsupportedActuatorKind { .. } => "unsupported_actuator_kind",
            Self::DuplicateControlBinding => "duplicate_control_binding",
            Self::ControlKindNotEligible { .. } => "control_kind_not_eligible",
            Self::SafetyRoleMustBeRequired { .. } => "safety_role_must_be_required",
            Self::LastControlBindingWhileAutomationEnabled => {
                "last_control_binding_while_automation_enabled"
            }
        }
    }
}

impl core::fmt::Display for BindingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UndeclaredSensor {
                device_id,
                sensor_id,
            } => write!(f, "device {device_id} never declared a sensor {sensor_id}"),
            Self::SensorDoesNotProduceKind { sensor_id, kind } => {
                write!(
                    f,
                    "sensor {sensor_id} does not produce {kind} at that point"
                )
            }
            Self::UndeclaredActuator {
                device_id,
                actuator_id,
            } => write!(
                f,
                "device {device_id} never declared an actuator {actuator_id}"
            ),
            Self::UnsupportedActuatorKind { kind } => {
                write!(
                    f,
                    "actuator kind {kind} cannot be bound; v1 waters with an irrigation_pump"
                )
            }
            Self::DuplicateControlBinding => {
                write!(f, "a plant may have at most one control binding")
            }
            Self::ControlKindNotEligible { kind } => write!(
                f,
                "{kind} cannot be a control measurement; a control kind must be a recognised scalar"
            ),
            Self::SafetyRoleMustBeRequired { kind } => write!(
                f,
                "{kind} must be bound as `required` while the plant has an actuator; \
                 marking it advisory would remove its veto"
            ),
            Self::LastControlBindingWhileAutomationEnabled => write!(
                f,
                "disable automatic watering before removing the last control binding"
            ),
        }
    }
}

/// Kinds whose role is fixed by safety rather than by preference.
///
/// Both are hard vetoes in the actuation gate, so neither may be demoted to
/// `advisory` on a plant that can actually water.
#[must_use]
pub fn is_safety_kind(kind: &MeasurementKind) -> bool {
    matches!(
        kind,
        MeasurementKind::LeakState | MeasurementKind::TankLevel
    )
}

/// Validates one sensor binding against declarations and the plant's other
/// bindings.
///
/// `existing` excludes the binding being validated, so an edit re-validates
/// cleanly. `plant_has_actuator` decides whether the leak/tank role rule applies.
///
/// # Errors
///
/// Returns the first violated rule.
pub fn validate_sensor_binding(
    binding: &SensorBinding,
    declared: &DeclaredCapabilities,
    existing: &[SensorBinding],
    plant_has_actuator: bool,
) -> Result<(), BindingError> {
    let device = binding.device_id.to_string();
    let sensor = binding.sensor_id.as_str();
    let matched = declared
        .sensors
        .iter()
        .find(|s| s.device_id == device && s.sensor_id == sensor)
        .ok_or_else(|| BindingError::UndeclaredSensor {
            device_id: device.clone(),
            sensor_id: sensor.to_owned(),
        })?;
    if !matched.kinds.contains(&binding.kind) || matched.point != binding.point.as_str() {
        return Err(BindingError::SensorDoesNotProduceKind {
            sensor_id: sensor.to_owned(),
            kind: binding.kind.as_str().to_owned(),
        });
    }
    if binding.role == BindingRole::Control {
        if !binding.kind.control_eligible() {
            return Err(BindingError::ControlKindNotEligible {
                kind: binding.kind.as_str().to_owned(),
            });
        }
        if existing.iter().any(|b| b.role == BindingRole::Control) {
            return Err(BindingError::DuplicateControlBinding);
        }
    }
    if plant_has_actuator && is_safety_kind(&binding.kind) && binding.role != BindingRole::Required
    {
        return Err(BindingError::SafetyRoleMustBeRequired {
            kind: binding.kind.as_str().to_owned(),
        });
    }
    Ok(())
}

/// Validates the optional actuator binding.
///
/// # Errors
///
/// Returns the first violated rule.
pub fn validate_actuator_binding(
    binding: &ActuatorBinding,
    declared: &DeclaredCapabilities,
) -> Result<(), BindingError> {
    let device = binding.device_id.to_string();
    let actuator = binding.actuator_id.as_str();
    let matched = declared
        .actuators
        .iter()
        .find(|a| a.device_id == device && a.actuator_id == actuator)
        .ok_or_else(|| BindingError::UndeclaredActuator {
            device_id: device.clone(),
            actuator_id: actuator.to_owned(),
        })?;
    if matched.kind != binding.kind || binding.kind != ActuatorKind::IrrigationPump {
        return Err(BindingError::UnsupportedActuatorKind {
            kind: format!("{:?}", binding.kind),
        });
    }
    Ok(())
}

/// Validates removing a sensor binding.
///
/// Removing the last `control` binding while automation is enabled leaves a
/// plant that is authorised to water and has nothing to decide with. Refuse it
/// and make the operator disable automation first, which is a decision rather
/// than a side effect.
///
/// # Errors
///
/// Returns [`BindingError::LastControlBindingWhileAutomationEnabled`].
pub fn validate_sensor_binding_removal(
    removed: &SensorBinding,
    remaining: &[SensorBinding],
    automation_enabled: bool,
) -> Result<(), BindingError> {
    if removed.role == BindingRole::Control
        && automation_enabled
        && !remaining.iter().any(|b| b.role == BindingRole::Control)
    {
        return Err(BindingError::LastControlBindingWhileAutomationEnabled);
    }
    Ok(())
}

/// Every kind the plant treats as safety-gating, derived from its `required`
/// bindings. This is what M5-016 turns into a policy's required measurements.
#[must_use]
pub fn required_kinds(bindings: &[SensorBinding]) -> Vec<MeasurementKind> {
    let mut kinds: Vec<MeasurementKind> = bindings
        .iter()
        .filter(|b| b.role == BindingRole::Required)
        .map(|b| b.kind.clone())
        .collect();
    kinds.dedup_by(|a, b| a == b);
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhizo_mqtt_contract::DeviceId;
    use rhizo_mqtt_contract::payload::{MeasurementPoint, SensorId};

    fn declared() -> DeclaredCapabilities {
        DeclaredCapabilities {
            sensors: vec![
                DeclaredSensor {
                    device_id: "plant-node-01".into(),
                    sensor_id: "soil-0".into(),
                    point: "default".into(),
                    kinds: vec![
                        MeasurementKind::SoilMoisture,
                        MeasurementKind::SoilTemperature,
                        MeasurementKind::SoilEc,
                    ],
                },
                DeclaredSensor {
                    device_id: "plant-node-01".into(),
                    sensor_id: "leak-0".into(),
                    point: "tray".into(),
                    kinds: vec![MeasurementKind::LeakState],
                },
                DeclaredSensor {
                    device_id: "plant-node-01".into(),
                    sensor_id: "tank-0".into(),
                    point: "reservoir".into(),
                    kinds: vec![MeasurementKind::TankLevel],
                },
                DeclaredSensor {
                    device_id: "plant-node-02".into(),
                    sensor_id: "soil-0".into(),
                    point: "default".into(),
                    kinds: vec![MeasurementKind::SoilMoisture],
                },
            ],
            actuators: vec![DeclaredActuator {
                device_id: "plant-node-01".into(),
                actuator_id: "pump-0".into(),
                kind: ActuatorKind::IrrigationPump,
            }],
        }
    }

    fn binding(
        sensor: &str,
        point: &str,
        kind: MeasurementKind,
        role: BindingRole,
    ) -> SensorBinding {
        SensorBinding {
            device_id: DeviceId::parse("plant-node-01").unwrap(),
            sensor_id: SensorId::parse(sensor).unwrap(),
            point: MeasurementPoint::parse(point).unwrap(),
            kind,
            role,
        }
    }

    #[test]
    fn a_binding_naming_an_undeclared_capability_is_rejected() {
        let missing = binding(
            "soil-9",
            "default",
            MeasurementKind::SoilMoisture,
            BindingRole::Control,
        );
        assert_eq!(
            validate_sensor_binding(&missing, &declared(), &[], true),
            Err(BindingError::UndeclaredSensor {
                device_id: "plant-node-01".into(),
                sensor_id: "soil-9".into()
            })
        );
        let wrong_kind = binding(
            "leak-0",
            "tray",
            MeasurementKind::SoilMoisture,
            BindingRole::Advisory,
        );
        assert_eq!(
            validate_sensor_binding(&wrong_kind, &declared(), &[], false),
            Err(BindingError::SensorDoesNotProduceKind {
                sensor_id: "leak-0".into(),
                kind: "soil_moisture".into()
            })
        );
        let wrong_point = binding(
            "soil-0",
            "tray",
            MeasurementKind::SoilMoisture,
            BindingRole::Control,
        );
        assert!(validate_sensor_binding(&wrong_point, &declared(), &[], false).is_err());
    }

    #[test]
    fn at_most_one_control_binding_per_plant() {
        let control = binding(
            "soil-0",
            "default",
            MeasurementKind::SoilMoisture,
            BindingRole::Control,
        );
        assert_eq!(
            validate_sensor_binding(&control, &declared(), &[], false),
            Ok(())
        );
        assert_eq!(
            validate_sensor_binding(&control, &declared(), std::slice::from_ref(&control), false),
            Err(BindingError::DuplicateControlBinding)
        );
    }

    #[test]
    fn a_control_binding_must_be_a_recognised_scalar_kind() {
        let leak_control = binding(
            "leak-0",
            "tray",
            MeasurementKind::LeakState,
            BindingRole::Control,
        );
        assert_eq!(
            validate_sensor_binding(&leak_control, &declared(), &[], false),
            Err(BindingError::ControlKindNotEligible {
                kind: "leak_state".into()
            })
        );
    }

    /// Demoting a veto to advisory would remove it silently, which is the
    /// failure the role model exists to prevent.
    #[test]
    fn leak_and_tank_cannot_be_advisory_when_an_actuator_exists() {
        for (sensor, point, kind) in [
            ("leak-0", "tray", MeasurementKind::LeakState),
            ("tank-0", "reservoir", MeasurementKind::TankLevel),
        ] {
            let advisory = binding(sensor, point, kind.clone(), BindingRole::Advisory);
            assert_eq!(
                validate_sensor_binding(&advisory, &declared(), &[], true),
                Err(BindingError::SafetyRoleMustBeRequired {
                    kind: kind.as_str().to_owned()
                })
            );
            let required = binding(sensor, point, kind.clone(), BindingRole::Required);
            assert_eq!(
                validate_sensor_binding(&required, &declared(), &[], true),
                Ok(())
            );
            // With no actuator there is nothing to veto, so the plant may
            // record a leak sensor advisorily and simply be alerted.
            assert_eq!(
                validate_sensor_binding(&advisory, &declared(), &[], false),
                Ok(())
            );
        }
    }

    #[test]
    fn the_optional_actuator_binding_must_name_a_declared_pump() {
        let good = ActuatorBinding {
            device_id: DeviceId::parse("plant-node-01").unwrap(),
            actuator_id: SensorId::parse("pump-0").unwrap(),
            kind: ActuatorKind::IrrigationPump,
        };
        assert_eq!(validate_actuator_binding(&good, &declared()), Ok(()));
        let missing = ActuatorBinding {
            actuator_id: SensorId::parse("pump-9").unwrap(),
            ..good.clone()
        };
        assert_eq!(
            validate_actuator_binding(&missing, &declared()),
            Err(BindingError::UndeclaredActuator {
                device_id: "plant-node-01".into(),
                actuator_id: "pump-9".into()
            })
        );
        let wrong_kind = ActuatorBinding {
            kind: ActuatorKind::GrowLight,
            ..good
        };
        assert!(matches!(
            validate_actuator_binding(&wrong_kind, &declared()),
            Err(BindingError::UnsupportedActuatorKind { .. })
        ));
    }

    #[test]
    fn removing_the_last_control_binding_is_refused_while_automation_is_on() {
        let control = binding(
            "soil-0",
            "default",
            MeasurementKind::SoilMoisture,
            BindingRole::Control,
        );
        assert_eq!(
            validate_sensor_binding_removal(&control, &[], true),
            Err(BindingError::LastControlBindingWhileAutomationEnabled)
        );
        assert_eq!(
            validate_sensor_binding_removal(&control, &[], false),
            Ok(())
        );
        let replacement = binding(
            "soil-0",
            "default",
            MeasurementKind::SoilMoisture,
            BindingRole::Control,
        );
        assert_eq!(
            validate_sensor_binding_removal(&control, &[replacement], true),
            Ok(())
        );
    }

    /// SCEN-106: a monitoring-only plant is fully configurable. Nothing here
    /// requires an actuator, and no rule turns its absence into an error.
    #[test]
    fn scen_106_a_monitoring_only_plant_binds_normally() {
        let bindings = [
            binding(
                "soil-0",
                "default",
                MeasurementKind::SoilMoisture,
                BindingRole::Control,
            ),
            binding(
                "soil-0",
                "default",
                MeasurementKind::SoilTemperature,
                BindingRole::Advisory,
            ),
        ];
        for (i, b) in bindings.iter().enumerate() {
            assert_eq!(
                validate_sensor_binding(b, &declared(), &bindings[..i], false),
                Ok(()),
                "{b:?}"
            );
        }
        assert!(required_kinds(&bindings).is_empty());
    }

    /// Replacing a probe is a binding edit: the kind, the role, and therefore
    /// every policy keyed on the kind are unchanged.
    #[test]
    fn replacing_a_sensor_preserves_the_kind_and_the_role() {
        let before = binding(
            "soil-0",
            "default",
            MeasurementKind::SoilMoisture,
            BindingRole::Control,
        );
        let after = SensorBinding {
            device_id: DeviceId::parse("plant-node-02").unwrap(),
            ..before.clone()
        };
        assert_eq!(
            validate_sensor_binding(&after, &declared(), &[], false),
            Ok(())
        );
        assert_eq!(after.kind, before.kind);
        assert_eq!(after.role, before.role);
    }

    #[test]
    fn required_kinds_are_derived_from_required_roles_only() {
        let bindings = [
            binding(
                "soil-0",
                "default",
                MeasurementKind::SoilMoisture,
                BindingRole::Control,
            ),
            binding(
                "leak-0",
                "tray",
                MeasurementKind::LeakState,
                BindingRole::Required,
            ),
            binding(
                "tank-0",
                "reservoir",
                MeasurementKind::TankLevel,
                BindingRole::Required,
            ),
            binding(
                "soil-0",
                "default",
                MeasurementKind::SoilEc,
                BindingRole::Advisory,
            ),
        ];
        assert_eq!(
            required_kinds(&bindings),
            vec![MeasurementKind::LeakState, MeasurementKind::TankLevel]
        );
    }
}
