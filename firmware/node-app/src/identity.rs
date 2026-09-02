//! Device identity and boot identity (F-090-20, F-090-22, ADR-012).
//!
//! One firmware image serves every device: the identity is derived from the MAC
//! at first boot and persisted, and a serial-provisioned override wins over the
//! derived value. No binary contains a device name and no binary contains a
//! secret.

use rhizo_mqtt_contract::{
    BootId, DeviceId, UtcMillis,
    ids::{DeviceIdError, RandomSource},
};

use crate::persist::{BootIdentity, PersistedState};
use crate::ports::Rng;

/// The prefix every derived identity carries.
pub const DERIVED_PREFIX: &str = "plant-node-";

/// Derives `plant-node-<3-byte MAC hex>` from the interface MAC.
///
/// The low three bytes, which are the vendor-assigned half and therefore the
/// half that actually distinguishes two boards from the same batch.
///
/// # Errors
///
/// Only if the grammar of ADR-012 rejects the result, which it cannot for hex
/// digits — the signature carries the error anyway rather than unwrapping,
/// because `DeviceId::parse` is the one constructor and bypassing it here would
/// be the start of a second grammar.
pub fn derive_from_mac(mac: [u8; 6]) -> Result<DeviceId, DeviceIdError> {
    let suffix = format!("{:02x}{:02x}{:02x}", mac[3], mac[4], mac[5]);
    DeviceId::parse(&format!("{DERIVED_PREFIX}{suffix}"))
}

/// Resolves the identity this boot runs under.
///
/// A provisioned override wins; otherwise the MAC-derived value is used. An
/// override that fails the grammar is **ignored rather than fatal**: a device
/// that refuses to boot because somebody typed a capital letter over serial is
/// a device that needs a visit.
#[must_use]
pub fn resolve(state: &PersistedState, mac: [u8; 6]) -> Option<DeviceId> {
    state
        .provisioning
        .device_id
        .as_deref()
        .and_then(|value| DeviceId::parse(value).ok())
        .or_else(|| derive_from_mac(mac).ok())
}

/// Mints a fresh boot identity and advances the persisted generation.
///
/// `boot_id` is fresh each boot so the edge can tell one run from another;
/// `boot_generation` is monotone **across** reboots so retained statuses order
/// correctly even when two arrive out of sequence.
pub fn begin_boot(
    state: &mut PersistedState,
    now: Option<UtcMillis>,
    rng: &mut impl Rng,
) -> BootIdentity {
    state.boot_generation = state.boot_generation.saturating_add(1);
    let mut adapter = RngAdapter(rng);
    let boot_id = BootId::new_v7(now.unwrap_or(UtcMillis(0)), &mut adapter);
    BootIdentity {
        boot_id,
        boot_generation: state.boot_generation,
    }
}

/// Bridges the app's [`Rng`] to the contract's [`RandomSource`].
struct RngAdapter<'a, R: Rng + ?Sized>(&'a mut R);

impl<R: Rng + ?Sized> RandomSource for RngAdapter<'_, R> {
    fn fill_bytes(&mut self, output: &mut [u8]) {
        self.0.fill(output);
    }
}

/// Mints a message identifier, v7 when the clock is synchronised and v4 when it
/// is not (F-090-10 of PRD 020, protocol §4).
///
/// A v7 identifier embeds a timestamp, so minting one from an unsynchronised
/// clock would put a fabricated instant into an id the edge sorts by. v4 says
/// "no time here" honestly.
pub fn mint_message_id(
    now: Option<UtcMillis>,
    rng: &mut impl Rng,
) -> rhizo_mqtt_contract::MessageId {
    let mut adapter = RngAdapter(rng);
    match now {
        Some(now) => rhizo_mqtt_contract::MessageId::new_v7(now, &mut adapter),
        None => {
            let mut bytes = [0u8; 16];
            adapter.fill_bytes(&mut bytes);
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::from_bytes(bytes))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::CountingRng;

    #[test]
    fn identity_is_derived_from_the_low_three_mac_bytes() {
        let id = derive_from_mac([0x24, 0x6f, 0x28, 0xab, 0xcd, 0xef]).expect("valid");
        assert_eq!(id.as_ref(), "plant-node-abcdef");
    }

    #[test]
    fn a_provisioned_override_wins_over_the_derived_value() {
        let mut state = PersistedState::default();
        state.provisioning.device_id = Some("balcony-basil".into());
        let id = resolve(&state, [0; 6]).expect("resolves");
        assert_eq!(id.as_ref(), "balcony-basil");
    }

    #[test]
    fn an_ungrammatical_override_falls_back_rather_than_failing_to_boot() {
        let mut state = PersistedState::default();
        state.provisioning.device_id = Some("Balcony Basil".into());
        let id = resolve(&state, [0, 0, 0, 1, 2, 3]).expect("falls back");
        assert_eq!(id.as_ref(), "plant-node-010203");
    }

    #[test]
    fn boot_generation_is_monotone_and_boot_id_is_fresh() {
        let mut state = PersistedState::default();
        let mut rng = CountingRng::new(1);
        let first = begin_boot(&mut state, Some(UtcMillis(1_000)), &mut rng);
        let second = begin_boot(&mut state, Some(UtcMillis(2_000)), &mut rng);
        assert_eq!(first.boot_generation, 1);
        assert_eq!(second.boot_generation, 2);
        assert_ne!(first.boot_id, second.boot_id);
    }

    #[test]
    fn an_unsynced_clock_mints_v4_rather_than_a_fabricated_v7() {
        let mut rng = CountingRng::new(7);
        let synced = mint_message_id(Some(UtcMillis(1_700_000_000_000)), &mut rng);
        let unsynced = mint_message_id(None, &mut rng);
        assert_eq!(synced.as_uuid().get_version_num(), 7);
        assert_eq!(unsynced.as_uuid().get_version_num(), 4);
    }
}
