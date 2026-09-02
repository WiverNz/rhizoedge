//! Persistent state on ESP-IDF NVS (M9-004).
//!
//! # Two slots and a checksum, not one blob
//!
//! The state is written to whichever of two slots is not currently active, and
//! the active slot is then switched. A power cut during a write therefore
//! leaves the *previous* complete state intact — the same shape the policy
//! store uses for the same reason (SAFETY-019). Each slot carries a CRC-32 of
//! its own bytes.
//!
//! # A corrupt store loads nothing
//!
//! `load()` returns `None` rather than a partially decoded state. The caller
//! starts fresh, logs, and publishes `nvs_reset` — and treats an empty dedup
//! ring as *no evidence about past commands*, never as a clean slate.
//!
//! # Flash wear
//!
//! The dedup ring is written on every dose and the pending-result ledger on
//! every result. Doses are infrequent, so this is acceptable, and it is noted
//! rather than hidden. What is **not** acceptable is writing the per-wake
//! accounting here: at roughly 96 wakes a day the NVS write budget becomes the
//! limiting component of the device, which is why that lives in RTC memory
//! (ADR-018 §6, F-090-60).

use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use rhizo_node_app::persist::{crc32, PersistedState};
use rhizo_node_app::ports::{NvsError, NvsStore};

/// The namespace every key lives under.
pub const NAMESPACE: &str = "rhizo";

/// The two state slots, written alternately.
const SLOTS: [&str; 2] = ["state_a", "state_b"];
/// Which slot is current.
const ACTIVE_KEY: &str = "state_active";

/// The largest state blob a slot will hold.
///
/// Sized for a full dedup ring, a full pending-result ledger, a policy set, and
/// a full event buffer with room to spare. A state that does not fit is a bug
/// in those bounds, not a reason to truncate: `store` returns
/// [`NvsError::OutOfSpace`] and the caller aborts whatever it was about to do.
const MAX_BLOB: usize = 32 * 1024;

/// ESP-IDF NVS with two CRC-protected slots.
pub struct EspNvsStore {
    nvs: EspNvs<NvsDefault>,
    active: u8,
}

impl EspNvsStore {
    /// Opens the namespace read-write.
    ///
    /// # Errors
    ///
    /// If the partition or the namespace cannot be opened.
    pub fn new(partition: EspDefaultNvsPartition) -> Result<Self, esp_idf_sys::EspError> {
        let nvs = EspNvs::new(partition, NAMESPACE, true)?;
        let active = nvs.get_u8(ACTIVE_KEY).ok().flatten().unwrap_or(0) % 2;
        Ok(Self { nvs, active })
    }

    /// Which slot is current, for logging.
    #[must_use]
    pub const fn active_slot(&self) -> u8 {
        self.active
    }

    fn read_slot(&self, slot: u8) -> Option<PersistedState> {
        let mut buffer = vec![0u8; MAX_BLOB];
        let bytes = self
            .nvs
            .get_blob(SLOTS[usize::from(slot % 2)], &mut buffer)
            .ok()
            .flatten()?;
        if bytes.len() < 4 {
            return None;
        }
        let split = bytes.len() - 4;
        let (payload, stored) = bytes.split_at(split);
        let expected = u32::from_le_bytes([stored[0], stored[1], stored[2], stored[3]]);
        if crc32(payload) != expected {
            // The bytes still *look* like state, which is the more alarming of
            // the two failures: without the checksum the damage would have been
            // applied silently.
            log::error!("nvs slot {slot} failed its checksum; refusing to use it");
            return None;
        }
        serde_json::from_slice(payload).ok()
    }
}

impl NvsStore for EspNvsStore {
    fn load(&self) -> Option<PersistedState> {
        // The active slot first, then the other one: a crash between the write
        // and the pointer flip leaves the older slot current and complete, and
        // an active slot that fails its checksum is a reason to fall back to
        // the previous *complete* state rather than to nothing.
        self.read_slot(self.active)
            .or_else(|| self.read_slot(1 - self.active))
    }

    fn store(&mut self, state: &PersistedState) -> Result<(), NvsError> {
        let mut payload = serde_json::to_vec(state).map_err(|_| NvsError::WriteFailed)?;
        if payload.len() + 4 > MAX_BLOB {
            return Err(NvsError::OutOfSpace);
        }
        let checksum = crc32(&payload);
        payload.extend_from_slice(&checksum.to_le_bytes());

        let target = 1 - self.active;
        self.nvs
            .set_blob(SLOTS[usize::from(target)], &payload)
            .map_err(|_| NvsError::WriteFailed)?;
        // The one operation that makes the new state current. Everything before
        // it wrote only to the inactive slot.
        self.nvs
            .set_u8(ACTIVE_KEY, target)
            .map_err(|_| NvsError::WriteFailed)?;
        self.active = target;
        Ok(())
    }
}
