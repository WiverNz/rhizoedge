//! Explicit bench telemetry stays valid and advisory.
#![allow(clippy::unwrap_used)]
use rhizo_mqtt_contract::payload::{Quality, SensorConfig};
use rhizo_node_app::telemetry;

#[test]
fn configured_bench_adapters_produce_valid_advisory_samples() {
    let config = SensorConfig {
        soil: true,
        tank: true,
        weight: true,
        leak: true,
    };
    let (batch, errors) = telemetry::bench_sample(uuid::Uuid::nil(), config);
    assert!(batch.validate().is_ok());
    assert_eq!(batch.samples.len(), 4);
    assert_eq!(errors.total(), 1);
    for sample in &batch.samples {
        assert!(sample.validate().is_valid());
        assert_eq!(sample.point.as_str(), "m9-bench");
        assert!(
            sample.advisory_only(),
            "bench data must not authorise watering"
        );
        assert_ne!(sample.quality, Quality::Ok);
    }
    let fault = batch
        .samples
        .iter()
        .find(|s| s.quality == Quality::Fault)
        .unwrap();
    assert!(fault.value.is_none());
}

#[test]
fn disabled_bench_sensors_are_not_invented() {
    let (batch, errors) = telemetry::bench_sample(uuid::Uuid::nil(), SensorConfig::default());
    assert!(batch.samples.is_empty());
    assert_eq!(errors.total(), 0);
}
