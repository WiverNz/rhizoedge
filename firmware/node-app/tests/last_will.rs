//! Exercise the actual ESP wire serializer without radio hardware.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "../../esp32-node/src/net/will.rs"]
mod will;

#[test]
fn reconnect_and_reboot_wills_have_distinct_complete_envelopes() {
    let id = rhizo_mqtt_contract::DeviceId::parse("plant-node-579814").unwrap();
    let mut rng = rhizo_node_app::fakes::CountingRng::new(7);
    let mut state = rhizo_node_app::persist::PersistedState::default();
    let boot = rhizo_node_app::identity::begin_boot(&mut state, None, &mut rng);
    let first: serde_json::Value =
        serde_json::from_str(&will::last_will_payload(&id, boot, &mut rng)).unwrap();
    let second: serde_json::Value =
        serde_json::from_str(&will::last_will_payload(&id, boot, &mut rng)).unwrap();
    assert_eq!(
        first["boot_id"], second["boot_id"],
        "reconnect retains boot identity"
    );
    let next_boot = rhizo_node_app::identity::begin_boot(&mut state, None, &mut rng);
    let third: serde_json::Value =
        serde_json::from_str(&will::last_will_payload(&id, next_boot, &mut rng)).unwrap();
    assert_ne!(first["message_id"], third["message_id"]);
    assert_ne!(first["boot_id"], third["boot_id"]);
    assert_eq!(third["data"]["boot_generation"], boot.boot_generation + 1);
    let typed: rhizo_mqtt_contract::Envelope<rhizo_mqtt_contract::payload::DeviceStatus> =
        rhizo_mqtt_contract::Envelope::from_json(first.to_string().as_bytes()).unwrap();
    typed
        .check_topic(&rhizo_mqtt_contract::Topic::Status(id))
        .unwrap();
    typed.data.validate().unwrap();
    assert_ne!(
        first["message_id"], second["message_id"],
        "Edge dedup must not discard every later will"
    );
    assert!(first.get("boot_id").is_some());
    assert_eq!(first["sequence"], 0);
    assert_eq!(first["clock_synced"], false);
    assert!(first.get("device_time_ms").is_none());
    assert_eq!(first["data"]["reason"], "connection_lost");
    assert!(first["data"].get("power").is_none());
}
