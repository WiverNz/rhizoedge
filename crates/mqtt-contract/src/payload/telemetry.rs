//! Typed extensible measurement batches from ADR-017.
use crate::validation::ValidationReport;
use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use serde::{Deserialize, Deserializer, Serialize, de};
use uuid::Uuid;

/// Known measurement kinds plus a preserved forward-compatible value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MeasurementKind {
    SoilMoisture,
    SoilTemperature,
    SoilEc,
    SoilPh,
    AmbientTemperature,
    AmbientHumidity,
    Illuminance,
    PotWeight,
    TankLevel,
    LeakState,
    NitrateConcentration,
    Unknown(String),
}
impl Serialize for MeasurementKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
impl MeasurementKind {
    /** Wire name. */
    pub fn as_str(&self) -> &str {
        match self {
            Self::SoilMoisture => "soil_moisture",
            Self::SoilTemperature => "soil_temperature",
            Self::SoilEc => "soil_ec",
            Self::SoilPh => "soil_ph",
            Self::AmbientTemperature => "ambient_temperature",
            Self::AmbientHumidity => "ambient_humidity",
            Self::Illuminance => "illuminance",
            Self::PotWeight => "pot_weight",
            Self::TankLevel => "tank_level",
            Self::LeakState => "leak_state",
            Self::NitrateConcentration => "nitrate_concentration",
            Self::Unknown(v) => v,
        }
    }
    /** Compile-time spec for known values; unknown values are advisory. */
    pub const fn spec(&self) -> Option<KindSpec> {
        match self {
            Self::SoilMoisture => Some(KindSpec::scalar(Unit::VwcPercent, 0.0, 100.0)),
            Self::SoilTemperature => Some(KindSpec::scalar(Unit::Celsius, -20.0, 80.0)),
            Self::SoilEc => Some(KindSpec::scalar(Unit::UsCm, 0.0, 20_000.0)),
            Self::SoilPh => Some(KindSpec::scalar(Unit::Ph, 0.0, 14.0)),
            Self::AmbientTemperature => Some(KindSpec::scalar(Unit::Celsius, -40.0, 85.0)),
            Self::AmbientHumidity => Some(KindSpec::scalar(Unit::PercentRh, 0.0, 100.0)),
            Self::Illuminance => Some(KindSpec::scalar(Unit::Lux, 0.0, 200_000.0)),
            Self::PotWeight => Some(KindSpec::scalar(Unit::Gram, 0.0, 100_000.0)),
            Self::TankLevel => Some(KindSpec::scalar(Unit::Percent, 0.0, 100.0)),
            Self::LeakState => Some(KindSpec {
                unit: Unit::Boolean,
                class: MeasurementClass::Boolean,
                min: 0.0,
                max: 1.0,
            }),
            Self::NitrateConcentration => Some(KindSpec::scalar(Unit::MgL, 0.0, 5000.0)),
            Self::Unknown(_) => None,
        }
    }
    /** Whether this kind may provide actuation evidence. */
    pub const fn control_eligible(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }
}
impl<'de> Deserialize<'de> for MeasurementKind {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "soil_moisture" => Self::SoilMoisture,
            "soil_temperature" => Self::SoilTemperature,
            "soil_ec" => Self::SoilEc,
            "soil_ph" => Self::SoilPh,
            "ambient_temperature" => Self::AmbientTemperature,
            "ambient_humidity" => Self::AmbientHumidity,
            "illuminance" => Self::Illuminance,
            "pot_weight" => Self::PotWeight,
            "tank_level" => Self::TankLevel,
            "leak_state" => Self::LeakState,
            "nitrate_concentration" => Self::NitrateConcentration,
            _ => Self::Unknown(s),
        })
    }
}
/// Canonical measurement unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    VwcPercent,
    Celsius,
    UsCm,
    Ph,
    PercentRh,
    Lux,
    Gram,
    Percent,
    Boolean,
    MgL,
}
/// Scalar versus boolean measurement class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeasurementClass {
    Scalar,
    Boolean,
}
/// Compile-time kind metadata.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KindSpec {
    /** Canonical unit. */
    pub unit: Unit,
    /** Value class. */
    pub class: MeasurementClass,
    /** Inclusive scalar minimum. */
    pub min: f64,
    /** Inclusive scalar maximum. */
    pub max: f64,
}
impl KindSpec {
    const fn scalar(unit: Unit, min: f64, max: f64) -> Self {
        Self {
            unit,
            class: MeasurementClass::Scalar,
            min,
            max,
        }
    }
}
/// Typed measurement value. `None` in a sample is a failed read.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MeasurementValue {
    Scalar(f64),
    Boolean(bool),
}
/// Measurement quality.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Ok,
    Uncalibrated,
    Suspect,
    Fault,
}

macro_rules! validated_string {
    ($name:ident,$label:literal) => {
        #[doc=$label]
        #[derive(Clone, Debug, Eq, PartialEq, Serialize)]
        pub struct $name(String);
        impl $name {
            /** Parses the MQTT-safe local identifier. */
            pub fn parse(s: &str) -> Result<Self, LocalIdError> {
                if s.is_empty()
                    || s.len() > 32
                    || !s.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'
                    })
                {
                    Err(LocalIdError)
                } else {
                    Ok(Self(s.to_string()))
                }
            }
            /** String value. */
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                Self::parse(&s).map_err(|_| de::Error::custom("invalid local identifier"))
            }
        }
    };
}
/// Invalid point/sensor/calibration reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalIdError;
validated_string!(MeasurementPoint, "A measurement location within a device.");
validated_string!(SensorId, "A stable sensor capability identifier.");
validated_string!(CalibrationRef, "An opaque calibration-record reference.");
fn default_point() -> MeasurementPoint {
    MeasurementPoint("default".to_string())
}
/// One typed sample in a sampling cycle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasurementSample {
    #[serde(default = "default_point")]
    /** Measurement point. */
    pub point: MeasurementPoint,
    /** Typed kind. */
    pub kind: MeasurementKind,
    /** Value or null on read failure. */
    pub value: Option<MeasurementValue>,
    /** Canonical consistency check. */
    pub unit: Unit,
    /** Sensor quality. */
    pub quality: Quality,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Declared sensor. */
    pub sensor_id: Option<SensorId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Calibration record. */
    pub calibration_ref: Option<CalibrationRef>,
}
impl MeasurementSample {
    /** Validates unit, class, physical bounds, and fault-null semantics. */
    pub fn validate(&self) -> ValidationReport {
        let mut r = ValidationReport::default();
        if let Some(spec) = self.kind.spec() {
            if self.unit != spec.unit {
                r.push("unit");
            }
            match (self.value, spec.class) {
                (Some(MeasurementValue::Scalar(v)), MeasurementClass::Scalar)
                    if v.is_finite() && v >= spec.min && v <= spec.max => {}
                (Some(MeasurementValue::Boolean(_)), MeasurementClass::Boolean) => {}
                (None, _) if self.quality == Quality::Fault => {}
                _ => r.push("value"),
            }
        }
        r
    }
    /** Unknown kinds are always advisory. */
    pub const fn advisory_only(&self) -> bool {
        !self.kind.control_eligible() || !matches!(self.quality, Quality::Ok)
    }
}
/// One atomic sampling-cycle batch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TelemetryBatch {
    /** Cycle UUID. */
    pub batch_id: Uuid,
    /** One to 64 samples. */
    pub samples: Vec<MeasurementSample>,
}
/// Batch structural validation error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BatchError {
    Empty,
    TooMany,
}
impl TelemetryBatch {
    /** Validates only batch cardinality; samples remain partially usable. */
    pub fn validate(&self) -> Result<(), BatchError> {
        match self.samples.len() {
            0 => Err(BatchError::Empty),
            1..=64 => Ok(()),
            _ => Err(BatchError::TooMany),
        }
    }
}
/// Irrigation actuator state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActuatorState {
    /** Stable actuator id. */
    pub actuator_id: SensorId,
    /** Actuator kind. */
    pub kind: ActuatorKind,
    /** Currently active. */
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Previous run duration. */
    pub last_run_ms: Option<u32>,
    /** Volume in rolling device budget. */
    pub delivered_today_ml: f32,
    /** Hardware fault. */
    pub faulted: bool,
}
/// Declared actuator capability, including reserved expansion points.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActuatorKind {
    IrrigationPump,
    Valve,
    GrowLight,
    Fan,
    Heater,
    Humidifier,
    FertiliserDosingPump,
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(kind: MeasurementKind, value: MeasurementValue, unit: Unit) -> MeasurementSample {
        MeasurementSample {
            point: default_point(),
            kind,
            value: Some(value),
            unit,
            quality: Quality::Ok,
            sensor_id: None,
            calibration_ref: None,
        }
    }
    #[test]
    fn all_specs_and_boundaries() {
        let cases = [
            (MeasurementKind::SoilMoisture, Unit::VwcPercent, 0., 100.),
            (MeasurementKind::SoilTemperature, Unit::Celsius, -20., 80.),
            (MeasurementKind::SoilEc, Unit::UsCm, 0., 20000.),
            (MeasurementKind::SoilPh, Unit::Ph, 0., 14.),
            (
                MeasurementKind::AmbientTemperature,
                Unit::Celsius,
                -40.,
                85.,
            ),
            (MeasurementKind::AmbientHumidity, Unit::PercentRh, 0., 100.),
            (MeasurementKind::Illuminance, Unit::Lux, 0., 200000.),
            (MeasurementKind::PotWeight, Unit::Gram, 0., 100000.),
            (MeasurementKind::TankLevel, Unit::Percent, 0., 100.),
            (MeasurementKind::NitrateConcentration, Unit::MgL, 0., 5000.),
        ];
        for (k, u, min, max) in cases {
            assert!(
                sample(k.clone(), MeasurementValue::Scalar(min), u)
                    .validate()
                    .is_valid()
            );
            assert!(
                sample(k.clone(), MeasurementValue::Scalar(max), u)
                    .validate()
                    .is_valid()
            );
            assert!(
                !sample(k, MeasurementValue::Scalar(max + 0.1), u)
                    .validate()
                    .is_valid()
            );
        }
    }
    #[test]
    fn unknown_is_preserved_and_advisory() {
        let s: MeasurementSample = serde_json::from_str(
            r#"{"kind":"future_sensor","value":4.0,"unit":"lux","quality":"ok"}"#,
        )
        .unwrap();
        assert!(matches!(s.kind,MeasurementKind::Unknown(ref v)if v=="future_sensor"));
        assert!(s.advisory_only());
    }
    #[test]
    fn boolean_scalar_and_nonfinite_are_distinct() {
        assert!(
            sample(
                MeasurementKind::LeakState,
                MeasurementValue::Boolean(false),
                Unit::Boolean
            )
            .validate()
            .is_valid()
        );
        assert!(
            !sample(
                MeasurementKind::LeakState,
                MeasurementValue::Scalar(0.),
                Unit::Boolean
            )
            .validate()
            .is_valid()
        );
        assert!(
            !sample(
                MeasurementKind::SoilMoisture,
                MeasurementValue::Scalar(f64::NAN),
                Unit::VwcPercent
            )
            .validate()
            .is_valid()
        );
    }
}
