//! Configuration-selected soil adapters and the SEN0601 read-only RTU subset.
use crate::ports::{SensorError, SoilReading, SoilSensor};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, rc::Rc};

/// One bounded RTU transaction. The transport returns the actual response length.
pub type Exchange = dyn FnMut(&[u8; 8], &mut [u8; 11]) -> Result<usize, SensorError>;

/// Local hardware configuration, persisted by the existing provisioning console.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoilConfig {
    /// `none`, `fake`, or `sen0601`. Unknown values fail boot validation.
    pub kind: String,
    /// One to three unique device addresses (SEN0601 supports 1..=254).
    pub addresses: Vec<u8>,
}
impl Default for SoilConfig {
    fn default() -> Self {
        Self {
            kind: "none".into(),
            addresses: vec![1],
        }
    }
}
impl SoilConfig {
    /// Reject unsupported adapters and ambiguous bus configurations.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !matches!(self.kind.as_str(), "none" | "fake" | "sen0601") {
            return Err("unknown soil adapter: use none, fake, or sen0601");
        }
        if self.addresses.is_empty()
            || self.addresses.len() > 3
            || self
                .addresses
                .iter()
                .enumerate()
                .any(|(i, a)| *a == 0 || *a == 255 || self.addresses[..i].contains(a))
        {
            return Err("soil requires 1-3 unique addresses in 1..=254");
        }
        Ok(())
    }
}

/// Modbus CRC-16, transmitted low byte first.
#[must_use]
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for byte in bytes {
        crc ^= u16::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xa001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// Read moisture, signed temperature and EC at 0x0000..=0x0002.
#[must_use]
pub fn request(address: u8) -> [u8; 8] {
    let mut frame = [address, 3, 0, 0, 0, 3, 0, 0];
    let crc = crc16(&frame[..6]).to_le_bytes();
    frame[6..].copy_from_slice(&crc);
    frame
}

/// Validate the entire response before exposing any measurement.
pub fn decode(address: u8, frame: &[u8]) -> Result<SoilReading, SensorError> {
    if frame.len() != 11
        || frame[0] != address
        || frame[1] != 3
        || frame[2] != 6
        || crc16(&frame[..9]) != u16::from_le_bytes([frame[9], frame[10]])
    {
        return Err(SensorError::ReadFailed);
    }
    let moisture = u16::from_be_bytes([frame[3], frame[4]]);
    let temperature = i16::from_be_bytes([frame[5], frame[6]]);
    if moisture > 1000 || !(-400..=800).contains(&temperature) {
        return Err(SensorError::ReadFailed);
    }
    Ok(SoilReading {
        vwc_percent: f32::from(moisture) / 10.0,
        temperature_c: Some(f32::from(temperature) / 10.0),
        ec_us_cm: Some(f32::from(u16::from_be_bytes([frame[7], frame[8]]))),
    })
}

struct Sen0601 {
    address: u8,
    bus: Rc<RefCell<Box<Exchange>>>,
}
impl SoilSensor for Sen0601 {
    fn read(&mut self) -> Result<SoilReading, SensorError> {
        let mut response = [0; 11];
        let len = (self.bus.borrow_mut())(&request(self.address), &mut response)?;
        decode(
            self.address,
            response.get(..len).ok_or(SensorError::ReadFailed)?,
        )
    }
}

/// Selected adapters, sharing a single serial bus and the existing telemetry path.
pub struct SoilAdapters {
    config: SoilConfig,
    sensors: Vec<Box<dyn SoilSensor>>,
    capabilities: Vec<rhizo_mqtt_contract::payload::SensorCapability>,
}
impl SoilAdapters {
    /// Construct once at boot. Fake/absent modes never require a bus.
    pub fn new(config: SoilConfig, bus: Option<Box<Exchange>>) -> Result<Self, &'static str> {
        config.validate()?;
        let mut sensors: Vec<Box<dyn SoilSensor>> = Vec::new();
        match config.kind.as_str() {
            "sen0601" => {
                let bus = Rc::new(RefCell::new(bus.ok_or("SEN0601 requires a board UART")?));
                for address in &config.addresses {
                    sensors.push(Box::new(Sen0601 {
                        address: *address,
                        bus: bus.clone(),
                    }));
                }
            }
            "fake" => {
                for _ in &config.addresses {
                    sensors.push(Box::new(crate::fakes::FakeSoil::reading(32.5)));
                }
            }
            _ => {}
        }
        use rhizo_mqtt_contract::payload::{
            MeasurementKind, MeasurementPoint, SensorCapability, SensorId,
        };
        let capabilities = config
            .addresses
            .iter()
            .take(sensors.len())
            .map(|address| {
                let id = format!("soil-{address}");
                SensorCapability {
                    sensor_id: SensorId::parse(&id)
                        .unwrap_or_else(|_| unreachable!("valid sensor id")),
                    point: MeasurementPoint::parse(&id)
                        .unwrap_or_else(|_| unreachable!("valid point")),
                    kinds: if config.kind == "sen0601" {
                        vec![
                            MeasurementKind::SoilMoisture,
                            MeasurementKind::SoilTemperature,
                            MeasurementKind::SoilEc,
                        ]
                    } else {
                        vec![MeasurementKind::SoilMoisture]
                    },
                    present: false,
                    healthy: false,
                    errors: 0,
                    calibrated: Some(false),
                }
            })
            .collect();
        Ok(Self {
            config,
            sensors,
            capabilities,
        })
    }

    /// Status declarations use the same IDs as the emitted samples.
    #[must_use]
    pub fn capabilities(&self) -> Vec<rhizo_mqtt_contract::payload::SensorCapability> {
        self.capabilities.clone()
    }

    /// Append soil samples to a regular batch; failures keep null/fault semantics.
    pub fn sample(
        &mut self,
        batch: &mut rhizo_mqtt_contract::payload::TelemetryBatch,
    ) -> crate::telemetry::SensorErrors {
        use rhizo_mqtt_contract::payload::{MeasurementKind, Quality, Unit};
        let mut errors = crate::telemetry::SensorErrors::default();
        for (capability, soil) in self.capabilities.iter_mut().zip(&mut self.sensors) {
            let mut sensors = crate::telemetry::Sensors {
                soil: Some(soil.as_mut()),
                tank: None,
                leak: None,
                scale: None,
                battery: None,
            };
            let (mut readings, current) =
                crate::telemetry::sample(batch.batch_id, &mut sensors, None);
            errors.read_failures += current.read_failures;
            errors.not_present += current.not_present;
            errors.warmup_incomplete += current.warmup_incomplete;
            capability.present = self.config.kind == "sen0601" && current.total() == 0;
            capability.healthy = capability.present;
            capability.errors = capability.errors.saturating_add(current.total());
            if self.config.kind == "sen0601" && current.total() > 0 {
                // The whole register read failed, including its companion channels.
                for (kind, unit) in [
                    (MeasurementKind::SoilTemperature, Unit::Celsius),
                    (MeasurementKind::SoilEc, Unit::UsCm),
                ] {
                    let mut failed = readings.samples[0].clone();
                    failed.kind = kind;
                    failed.unit = unit;
                    readings.samples.push(failed);
                }
            }
            for sample in &mut readings.samples {
                // Address-based points stay stable when sensors are reordered or removed.
                sample.point = capability.point.clone();
                sample.sensor_id = Some(capability.sensor_id.clone());
                // Manufacturer scaling is not a soil-specific VWC calibration.
                if sample.quality == Quality::Ok {
                    sample.quality = Quality::Uncalibrated;
                }
            }
            batch.samples.extend(readings.samples);
        }
        errors
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use rhizo_mqtt_contract::payload::{MeasurementValue, Quality, TelemetryBatch};
    fn response(address: u8, moisture: u16, temperature: i16, ec: u16) -> Vec<u8> {
        let mut frame = vec![address, 3, 6];
        frame.extend(moisture.to_be_bytes());
        frame.extend(temperature.to_be_bytes());
        frame.extend(ec.to_be_bytes());
        frame.extend(crc16(&frame).to_le_bytes());
        frame
    }
    fn config(kind: &str, addresses: &[u8]) -> SoilConfig {
        SoilConfig {
            kind: kind.into(),
            addresses: addresses.into(),
        }
    }
    fn batch() -> TelemetryBatch {
        TelemetryBatch {
            batch_id: uuid::Uuid::nil(),
            samples: vec![],
        }
    }
    #[test]
    fn register_scaling_and_signed_temperature() {
        assert_eq!(request(1), [1, 3, 0, 0, 0, 3, 5, 0xcb]);
        assert_eq!(
            decode(1, &response(1, 456, -123, 2345)).unwrap(),
            SoilReading {
                vwc_percent: 45.6,
                temperature_c: Some(-12.3),
                ec_us_cm: Some(2345.0)
            }
        );
        assert_eq!(
            decode(1, &response(1, 0, -400, 0)).unwrap().temperature_c,
            Some(-40.0)
        );
        assert_eq!(
            decode(1, &response(1, 1000, 800, 65535)).unwrap().ec_us_cm,
            Some(65535.0)
        );
        assert!(decode(1, &response(1, 1001, 100, 0)).is_err());
        assert!(decode(1, &response(1, 0, 801, 0)).is_err());
    }
    #[test]
    fn rejects_crc_address_function_length_and_exception_errors() {
        let frame = response(1, 300, 220, 100);
        for len in 0..frame.len() {
            assert!(decode(1, &frame[..len]).is_err());
        }
        let mut extra = frame.clone();
        extra.push(0);
        assert!(decode(1, &extra).is_err());
        for index in 0..frame.len() {
            let mut corrupt = frame.clone();
            corrupt[index] ^= 1;
            assert!(decode(1, &corrupt).is_err());
        }
        assert!(decode(2, &frame).is_err());
        for (index, value) in [(1, 4), (2, 4)] {
            let mut wrong = frame.clone();
            wrong[index] = value;
            let crc = crc16(&wrong[..9]).to_le_bytes();
            wrong[9..].copy_from_slice(&crc);
            assert!(decode(1, &wrong).is_err());
        }
        let mut exception = vec![1, 0x83, 2];
        exception.extend(crc16(&exception).to_le_bytes());
        assert!(decode(1, &exception).is_err());
    }
    #[test]
    fn selection_preserves_offline_fakes_and_rejects_invalid_configuration() {
        let mut absent = SoilAdapters::new(SoilConfig::default(), None).unwrap();
        let mut readings = batch();
        absent.sample(&mut readings);
        assert!(readings.samples.is_empty());
        let mut fake = SoilAdapters::new(config("fake", &[1]), None).unwrap();
        fake.sample(&mut readings);
        assert_eq!(
            readings.samples[0].value,
            Some(MeasurementValue::Scalar(32.5))
        );
        assert_eq!(readings.samples[0].quality, Quality::Uncalibrated);
        assert!(SoilAdapters::new(config("sen0601", &[1]), None).is_err());
        for cfg in [
            config("typo", &[1]),
            config("sen0601", &[]),
            config("sen0601", &[0]),
            config("sen0601", &[255]),
            config("sen0601", &[1, 1]),
            config("sen0601", &[1, 2, 3, 4]),
        ] {
            assert!(SoilAdapters::new(cfg, None).is_err());
        }
        assert!(config("sen0601", &[254]).validate().is_ok());
    }
    #[test]
    fn three_addresses_use_existing_telemetry_and_read_failure_is_not_stale_data() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let recorded = calls.clone();
        let exchange: Box<Exchange> = Box::new(move |request, output| {
            recorded.borrow_mut().push(request[0]);
            if request[0] == 2 {
                return Err(SensorError::ReadFailed);
            }
            output.copy_from_slice(&response(request[0], 420, -50, 1500));
            Ok(11)
        });
        let mut soil = SoilAdapters::new(config("sen0601", &[3, 2, 1]), Some(exchange)).unwrap();
        let mut readings = batch();
        assert_eq!(soil.sample(&mut readings).read_failures, 1);
        assert_eq!(*calls.borrow(), vec![3, 2, 1]);
        assert!(readings.validate().is_ok());
        assert!(
            readings
                .samples
                .iter()
                .all(|s| s.sensor_id.as_ref().unwrap().as_str() == s.point.as_str())
        );
        assert_eq!(soil.capabilities()[1].errors, 1);
        assert!(!soil.capabilities()[1].healthy);
        assert!(soil.capabilities()[0].healthy);
        assert_eq!(
            readings
                .samples
                .iter()
                .filter(|s| s.point.as_str() == "soil-2" && s.quality == Quality::Fault)
                .count(),
            3
        );
        let wire = serde_json::to_vec(&readings).unwrap();
        assert!(
            serde_json::from_slice::<TelemetryBatch>(&wire)
                .unwrap()
                .validate()
                .is_ok()
        );
        assert_eq!(
            readings.samples[1].value,
            Some(MeasurementValue::Scalar(-5.0))
        );
        let failed = readings
            .samples
            .iter()
            .find(|s| s.point.as_str() == "soil-2")
            .unwrap();
        assert_eq!((failed.value, failed.quality), (None, Quality::Fault));
        assert_eq!(readings.samples.last().unwrap().point.as_str(), "soil-1");
    }
    #[test]
    fn provisioning_round_trip_and_old_nvs_default() {
        use crate::{
            persist::{PersistedState, Provisioning},
            provision::{Console, ConsoleResponse},
        };
        assert_eq!(
            serde_json::from_str::<Provisioning>("{}").unwrap().soil,
            SoilConfig::default()
        );
        let mut state = PersistedState::default();
        let mut console = Console::open(&state);
        console.handle("provision soil sen0601 1 2 3", &mut state);
        assert_eq!(state.provisioning.soil.kind, "none");
        console.handle("provision commit", &mut state);
        assert_eq!(state.provisioning.soil, config("sen0601", &[1, 2, 3]));
        let saved = serde_json::to_string(&state.provisioning).unwrap();
        assert_eq!(
            serde_json::from_str::<Provisioning>(&saved).unwrap(),
            state.provisioning
        );
        assert!(console.redacted().contains("soil sen0601 [1, 2, 3]"));
        assert!(matches!(
            console.handle("provision soil invalid", &mut state),
            ConsoleResponse::Unknown(_)
        ));
        console.handle("provision commit", &mut state);
        assert_eq!(state.provisioning.soil, config("sen0601", &[1, 2, 3]));
        console.handle("provision soil sen0601", &mut state);
        console.handle("provision commit", &mut state);
        assert_eq!(state.provisioning.soil.addresses, vec![1]);
    }
}
