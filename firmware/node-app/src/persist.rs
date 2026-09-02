//! The NVS data model and its CRC-protected regions (PRD 090 §Data model).
//!
//! Two properties are load-bearing and are why this is one module rather than a
//! field on each consumer:
//!
//! * **A safety-critical region that fails its checksum refuses**, and never
//!   substitutes a default. A default threshold nobody authorised is exactly
//!   what SAFETY-013 forbids, and a cleared dedup ring is a licence to
//!   re-execute a command (SAFETY-001).
//! * **Activation is one atomic pointer flip.** Everything before it is
//!   non-destructive, so an interruption anywhere leaves the previous value
//!   intact; after it, the new value is complete (SAFETY-019, M9-015).

use serde::{Deserialize, Serialize};

use rhizo_mqtt_contract::{
    BootId, CommandId, UtcMillis,
    payload::{CommandResult, DeviceConfig, OfflinePolicySet},
};

use crate::budget::OfflineRuntime;
use crate::buffer::BufferState;
use crate::ledger::PendingLedger;

/// Credentials and identity written by serial provisioning (M9-006).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provisioning {
    /// Explicit identity override; absent means derive from the MAC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// Wi-Fi SSID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wifi_ssid: Option<String>,
    /// Wi-Fi pre-shared key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wifi_psk: Option<String>,
    /// Broker host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mqtt_host: Option<String>,
    /// Broker username.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mqtt_user: Option<String>,
    /// Broker password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mqtt_pass: Option<String>,
}

/// A dose recorded **before** actuation (F-090-34, SAFETY-011).
///
/// Its presence on boot is the only evidence that a dose was interrupted, so it
/// is written before the pump is energised and cleared only once the result is
/// durably ledgered.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InFlightDose {
    /// The command that authorised it, or the autonomous dose's own id.
    pub command_id: CommandId,
    /// Device wall time at start, where known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<UtcMillis>,
    /// Volume the device set out to deliver.
    pub requested_ml: f32,
    /// Whether the device authorised it itself while isolated.
    pub autonomous: bool,
}

/// One slot of the 16-entry command dedup ring (`COMMAND_DEDUP_RING`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DedupEntry {
    /// The executed command.
    pub command_id: CommandId,
    /// The result to re-publish if it is seen again.
    pub result: CommandResult,
}

/// The rolling device daily total (F-090-36).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DailyTotal {
    /// Volume delivered in the current day window.
    pub delivered_ml: f32,
    /// The day this total belongs to, as whole days since the epoch.
    pub day_epoch: u32,
}

/// Everything the device keeps across a power cycle.
///
/// Deliberately identical in content to the simulator's state file so restart
/// behaviour is comparable between them (PRD 020, PRD 090 §Data model) — with
/// the one documented exception of [`PendingLedger`]'s capacity and overflow
/// behaviour, which the simulator answers in a way that does not transfer to an
/// ESP32 (ADR-014 §Device-side pending-result ledger).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PersistedState {
    /// Serial-provisioned credentials and identity.
    #[serde(default)]
    pub provisioning: Provisioning,
    /// Monotone across reboot, for status ordering.
    #[serde(default)]
    pub boot_generation: u64,
    /// Last applied configuration version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_version: Option<u32>,
    /// Last applied configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<DeviceConfig>,
    /// Rolling device daily total.
    #[serde(default)]
    pub daily: DailyTotal,
    /// The 16-entry command dedup ring, oldest first.
    #[serde(default)]
    pub dedup_ring: Vec<DedupEntry>,
    /// A dose written before actuation and not yet completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_flight_dose: Option<InFlightDose>,
    /// Results the edge has not acknowledged (F-090-16 to F-090-19).
    #[serde(default)]
    pub pending_results: PendingLedger,
    /// The active offline policy set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_active: Option<VersionedPolicy>,
    /// A candidate policy set mid-update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_staging: Option<VersionedPolicy>,
    /// Conservative offline budget and cooldown state.
    #[serde(default)]
    pub offline_runtime: OfflineRuntime,
    /// Bounded tiered event buffer, gap marker, and replay progress.
    #[serde(default)]
    pub buffer: BufferState,
}

/// A policy set with the version and checksum that make activation atomic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VersionedPolicy {
    /// Edge-owned monotone version.
    pub policy_version: u32,
    /// The policy set itself.
    pub policies: OfflinePolicySet,
    /// CRC-32 of the canonical encoding of `policies`.
    pub crc32: u32,
}

impl VersionedPolicy {
    /// Seals a policy set with its checksum.
    #[must_use]
    pub fn seal(policy_version: u32, policies: OfflinePolicySet) -> Self {
        let crc32 = policy_crc(&policies);
        Self {
            policy_version,
            policies,
            crc32,
        }
    }

    /// Whether the stored checksum still matches the stored policies.
    #[must_use]
    pub fn checksum_valid(&self) -> bool {
        policy_crc(&self.policies) == self.crc32
    }
}

/// CRC-32 (IEEE) over the canonical JSON encoding of a policy set.
///
/// JSON rather than a bespoke encoding because the policy arrives as JSON and
/// `serde_json` is already linked; a second encoder would be a second thing to
/// keep in step with the wire.
#[must_use]
pub fn policy_crc(policies: &OfflinePolicySet) -> u32 {
    match serde_json::to_vec(policies) {
        Ok(bytes) => crc32(&bytes),
        // An unencodable policy is not a valid policy. `crc32(b"")` is 0 and no
        // encodable policy set encodes to nothing, so this can only ever fail a
        // comparison, never pass one.
        Err(_) => 0,
    }
}

/// CRC-32/IEEE, table-free.
///
/// Dependency-free on purpose: this links into flash on a device where every
/// crate costs space, and the checksum is the one thing that must not itself be
/// a supply-chain risk.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// The identity a boot runs under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootIdentity {
    /// Fresh each boot (F-090-22).
    pub boot_id: BootId,
    /// Monotone across reboot (protocol §5.5).
    pub boot_generation: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_known_vectors() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b"a"), 0xe8b7_be43);
    }

    #[test]
    fn state_round_trips_through_json() {
        let state = PersistedState {
            boot_generation: 7,
            ..PersistedState::default()
        };
        let encoded = serde_json::to_vec(&state).expect("state encodes");
        let decoded: PersistedState = serde_json::from_slice(&encoded).expect("state decodes");
        assert_eq!(state, decoded);
    }
}
