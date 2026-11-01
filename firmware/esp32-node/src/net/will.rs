//! Host-testable production Last Will serializer.
use rhizo_mqtt_contract::DeviceId;
use rhizo_node_app::{identity::mint_message_id, persist::BootIdentity, ports::Rng};

/// Builds the Last Will payload.
///
/// A Last Will declares neither a power mode nor a sleep: it is composed at
/// connect and delivered whenever the session drops, so it is evidence of an
/// absence and never a restatement of configuration. `reason:
/// "connection_lost"` is what makes an unannounced disappearance `isolated`
/// rather than `sleeping` (SAFETY-021).
#[must_use]
pub fn last_will_payload(
    device_id: &DeviceId,
    identity: BootIdentity,
    rng: &mut impl Rng,
) -> String {
    let status = serde_json::json!({
        "v": 1,
        "kind": "device.status",
        "message_id": mint_message_id(None, rng),
        "boot_id": identity.boot_id,
        "sequence": 0,
        "clock_synced": false,
        "device_id": device_id.as_ref(),
        "data": {
            "boot_generation": identity.boot_generation,
            "status": "offline",
            "reason": "connection_lost",
            "protocol_version": rhizo_mqtt_contract::PROTOCOL_VERSION,
        }
    });
    status.to_string()
}
