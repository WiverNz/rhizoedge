//! The NVS-equivalent persistent state file.
//!
//! PRD 020's data model mirrors what NVS holds on real hardware, deliberately:
//! it makes restart behaviour comparable between the simulator and the firmware,
//! which is what M9-014's conformance test relies on.
//!
//! # Corruption fails closed
//!
//! This is the rule the module exists for. Corrupt safety-critical state MUST
//! NOT become "start fresh": a device that forgets its dedup ring, its budget,
//! and its cooldown is a device that will happily water again immediately. So a
//! corrupt file produces a **persistent-state fault**: the simulator starts, it
//! keeps sensing and reporting, and actuation is disabled until the state is
//! explicitly recovered. Corruption can only ever make the device *less*
//! permissive — never more.
//!
//! Concretely, a corrupt load never:
//!
//! - clears deduplication or in-flight uncertainty,
//! - replenishes the daily budget or the rolling offline budget,
//! - shortens a cooldown,
//! - activates, or substitutes a default for, an offline policy.
//!
//! # The whole file is checksummed, not only the policy blob
//!
//! ADR-015 §7 asks for a CRC on the policy blob. That is not enough on its own:
//! JSON with one flipped digit is still valid JSON, so `"delivered_today_ml":
//! 460.0` silently becoming `60.0` would decode cleanly and hand the device
//! four hundred millilitres of budget it had already spent. A property test
//! found exactly that case. The file therefore carries a checksum over its
//! entire contents, and a mismatch is corruption — which fails closed like any
//! other corruption.
//!
//! # Writes are atomic
//!
//! Write to a temporary file, flush it to disk, then rename over the target.
//! Without this, `--fault restart-mid-dose` produces a truncated file and the
//! test then exercises the corrupt-file path instead of the interrupted-dose
//! path it was written to test — a test that passes for the wrong reason.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use rhizo_mqtt_contract::payload::{BufferedEvent, CommandResult, EventTier, OfflinePolicySet};
use rhizo_mqtt_contract::safety::{COMMAND_DEDUP_RING, FIRMWARE_MAX_DAILY_ML};
use rhizo_mqtt_contract::{CommandId, UtcMillis};
use serde::{Deserialize, Serialize};

/// Milliseconds in a day, for the daily hard-limit rollover.
const MS_PER_DAY: i64 = 86_400_000;

/// A dose the device began but has not recorded the end of.
///
/// Written **before** actuation (protocol §5.8 step 13), so a restart mid-dose
/// is detectable on the next boot. `started_at_ms` is the device's wall clock
/// when it had one; the monotonic instant is what the interruption logic uses,
/// because an isolated device may have no wall time at all.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InFlightDose {
    /// Which command is in flight.
    pub command_id: CommandId,
    /// Wall time at the start, if the clock was synchronised.
    #[serde(default)]
    pub started_at_ms: Option<UtcMillis>,
    /// Monotonic instant at the start, always meaningful.
    pub started_at_monotonic_ms: u64,
    /// What the edge asked for.
    pub requested_ml: f32,
    /// What the validator authorised after clamping.
    pub effective_ml: f32,
}

/// One entry of the dedup ring: a command and what it did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandRecord {
    /// The idempotency key the device deduplicates on.
    pub command_id: CommandId,
    /// The stored outcome, republished verbatim on a repeat.
    pub result: CommandResult,
}

/// A policy blob with the checksum that proves it was written whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredPolicy {
    /// The policy set as received.
    pub payload: OfflinePolicySet,
    /// Checksum of the canonical encoding of `payload`.
    pub checksum: String,
    /// Applied version per plant at the time of storage.
    #[serde(default)]
    pub versions: BTreeMap<String, u32>,
}

impl StoredPolicy {
    /// Stores a policy set with a freshly computed checksum.
    #[must_use]
    pub fn new(payload: OfflinePolicySet, versions: BTreeMap<String, u32>) -> Self {
        let checksum = checksum_of(&payload);
        Self {
            payload,
            checksum,
            versions,
        }
    }

    /// Whether the blob still matches its checksum.
    ///
    /// The read-back verification of ADR-015 §7 step 4. A blob that fails this
    /// is refused and **no default is substituted**.
    #[must_use]
    pub fn verify(&self) -> bool {
        checksum_of(&self.payload) == self.checksum
    }
}

/// Computes the stored checksum of a policy set.
///
/// CRC-32 rather than a cryptographic hash: ADR-015 §7 asks for a CRC, the
/// threat is a torn write rather than an adversary, and a table-free CRC costs
/// no dependency in a crate the firmware's structure is meant to mirror.
#[must_use]
pub fn checksum_of<T: Serialize>(value: &T) -> String {
    let encoded = serde_json::to_vec(value).unwrap_or_default();
    format!("crc32:{:08x}", crc32(&encoded))
}

/// CRC-32 (IEEE), computed bitwise so no table is carried.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// The rolling window an offline policy's budget is measured over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BudgetWindow {
    /// Monotonic time elapsed inside the current window.
    pub elapsed_ms: u64,
    /// Volume delivered inside the current window.
    pub delivered_ml: f32,
}

/// The monotonic state a later offline evaluator needs.
///
/// Persisted in M2 and read by M6-019; **nothing here is evaluated in M2**.
/// `cooldown_remaining_ms` is a *remaining duration*, never a wall-clock
/// deadline, precisely because the device may have no absolute time. On boot the
/// remaining duration is restored intact, so a reboot cannot shorten a cooldown
/// (SAFETY-015).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OfflineRuntime {
    /// Which phase of a watering cycle the device is in.
    ///
    /// Persisted because a reboot mid-cooldown must resume the cooldown, not
    /// start a fresh cycle (offline-autonomy.md §5).
    pub cycle: crate::offline_state::CyclePhase,
    /// Rolling budget accumulator.
    pub budget_window: BudgetWindow,
    /// How much cooldown is left.
    pub cooldown_remaining_ms: u64,
    /// Continuous time the control measurement has been below the trigger.
    pub confirmation_elapsed_ms: u64,
    /// Doses delivered in the current cycle.
    pub dose_count: u16,
}

/// Range of buffered history lost to eviction.
///
/// Carries its own identity and sequence so the `history.gap` event it becomes
/// is byte-identical on every replay, exactly like any other buffered event.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GapMetadata {
    /// The stable id the replayed marker carries.
    pub event_id: rhizo_mqtt_contract::EventId,
    /// The sequence the marker occupies.
    pub device_seq: u64,
    /// Monotonic instant the first loss occurred.
    pub monotonic_ms: u64,
    /// Wall time, if the clock was synchronised.
    #[serde(default)]
    pub device_time_ms: Option<UtcMillis>,
    /// First lost sequence.
    pub from_seq: u64,
    /// Last lost sequence.
    pub to_seq: u64,
    /// How many events were lost.
    pub lost_count: u32,
    /// Which tier lost them.
    pub lost_tier: EventTier,
}

/// The bounded device-side event buffer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EventBufferState {
    /// Buffered events, oldest first.
    #[serde(default)]
    pub events: Vec<BufferedEvent>,
    /// The next `device_seq` to hand out.
    #[serde(default)]
    pub next_seq: u64,
    /// Everything at or below this sequence has been acknowledged by the edge.
    #[serde(default)]
    pub pending_ack_through_seq: Option<u64>,
    /// Loss recorded since the last replay.
    #[serde(default)]
    pub gap: Option<GapMetadata>,
}

/// Why persisted state cannot be trusted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PersistentStateFault {
    /// Stable machine-readable reason.
    pub reason: String,
    /// Human-readable detail, including where the original file was kept.
    pub detail: String,
}

impl PersistentStateFault {
    /// The reason recorded when the state file could not be decoded.
    pub const CORRUPT: &'static str = "state_file_corrupt";
    /// The reason recorded when the state file could not be read at all.
    pub const UNREADABLE: &'static str = "state_file_unreadable";
    /// The reason recorded when the file decoded but its checksum disagreed.
    ///
    /// Distinct from [`CORRUPT`](Self::CORRUPT) because it is the more alarming
    /// of the two: the bytes still *look* like state, so without the checksum
    /// the damage would have been applied silently.
    pub const CHECKSUM: &'static str = "state_file_checksum_mismatch";
}

/// Everything that survives a restart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PersistentState {
    /// How many times this device has booted.
    pub boot_count: u64,
    /// The configuration version currently applied.
    pub applied_config_version: Option<u32>,
    /// Volume delivered against the compile-time daily cap.
    pub delivered_today_ml: f32,
    /// Which day that total belongs to, in days since the Unix epoch.
    pub delivered_day_epoch: Option<i64>,
    /// The last [`COMMAND_DEDUP_RING`] commands and their outcomes.
    pub command_ring: Vec<CommandRecord>,
    /// A dose begun but not completed.
    pub in_flight_dose: Option<InFlightDose>,
    /// Results that could not be published and must be republished after boot.
    pub pending_results: Vec<CommandResult>,
    /// The policy currently in force.
    pub policy_active: Option<StoredPolicy>,
    /// A candidate being written, before activation.
    pub policy_staging: Option<StoredPolicy>,
    /// Applied policy version per plant, reported in status.
    pub applied_policy_versions: BTreeMap<String, u32>,
    /// Monotonic evaluator state, persisted for M6-019.
    pub offline_runtime: OfflineRuntime,
    /// The bounded event buffer.
    pub offline_events: EventBufferState,
    /// Set when the stored state could not be trusted.
    pub persistent_state_fault: Option<PersistentStateFault>,
}

impl Default for PersistentState {
    fn default() -> Self {
        Self {
            boot_count: 0,
            applied_config_version: None,
            delivered_today_ml: 0.0,
            delivered_day_epoch: None,
            command_ring: Vec::new(),
            in_flight_dose: None,
            pending_results: Vec::new(),
            policy_active: None,
            policy_staging: None,
            applied_policy_versions: BTreeMap::new(),
            offline_runtime: OfflineRuntime::default(),
            offline_events: EventBufferState::default(),
            persistent_state_fault: None,
        }
    }
}

impl PersistentState {
    /// The state a device adopts when its stored state cannot be trusted.
    ///
    /// **Maximally restrictive by construction.** Every safety-relevant field
    /// takes the value that permits the least: the budget is spent, the cooldown
    /// is as long as it can be, the dose count is at its ceiling, and no policy
    /// is active. Actuation is refused outright while the fault stands, so these
    /// values are belt and braces — but they are the difference between a
    /// lockout that is enforced in one place and one that survives someone
    /// later removing that place.
    #[must_use]
    pub fn failed_closed(fault: PersistentStateFault) -> Self {
        Self {
            delivered_today_ml: FIRMWARE_MAX_DAILY_ML,
            offline_runtime: OfflineRuntime {
                cycle: crate::offline_state::CyclePhase::Idle,
                budget_window: BudgetWindow {
                    elapsed_ms: 0,
                    delivered_ml: FIRMWARE_MAX_DAILY_ML,
                },
                cooldown_remaining_ms: u64::MAX,
                confirmation_elapsed_ms: 0,
                dose_count: u16::MAX,
            },
            persistent_state_fault: Some(fault),
            ..Self::default()
        }
    }

    /// Whether the device may actuate at all.
    ///
    /// The one place that answers the question, so a caller cannot forget the
    /// fault. It is deliberately **not** part of `validate_water_command`: the
    /// validator is the shared wire-level gate, while this is a device-local
    /// condition that precedes it.
    #[must_use]
    pub const fn actuation_permitted(&self) -> bool {
        self.persistent_state_fault.is_none()
    }

    /// Finds a previous outcome for a command.
    #[must_use]
    pub fn previous(&self, command_id: CommandId) -> Option<&CommandRecord> {
        self.command_ring
            .iter()
            .find(|r| r.command_id == command_id)
    }

    /// Records an outcome, evicting the oldest entry beyond the ring size.
    pub fn record_command(&mut self, record: CommandRecord) {
        if let Some(existing) = self
            .command_ring
            .iter_mut()
            .find(|r| r.command_id == record.command_id)
        {
            *existing = record;
            return;
        }
        self.command_ring.push(record);
        while self.command_ring.len() > COMMAND_DEDUP_RING {
            self.command_ring.remove(0);
        }
    }

    /// Adds to the daily total, rolling the day over first if the clock allows.
    pub fn credit_delivery(&mut self, ml: f32, now: Option<UtcMillis>) {
        self.roll_day(now);
        self.delivered_today_ml = (self.delivered_today_ml + ml).max(0.0);
    }

    /// Resets the daily total when — and only when — a trustworthy clock shows
    /// a new day.
    ///
    /// An unsynchronised device does **not** roll the day over. It cannot prove
    /// a day has passed, and a device that cannot prove it is under budget must
    /// assume it is not: rolling over on a guess would hand out a fresh daily
    /// allowance to a device with a broken clock (SAFETY-015).
    pub fn roll_day(&mut self, now: Option<UtcMillis>) {
        let Some(now) = now else { return };
        let today = now.0.div_euclid(MS_PER_DAY);
        match self.delivered_day_epoch {
            Some(stored) if stored == today => {}
            Some(stored) if today < stored => {
                // The clock moved backwards. Keeping the higher total is the
                // conservative reading of a day that may not have ended.
            }
            _ => {
                self.delivered_day_epoch = Some(today);
                self.delivered_today_ml = 0.0;
            }
        }
    }
}

/// The on-disk envelope: a checksum and the state it covers.
///
/// A separate type from [`PersistentState`] so the checksum cannot accidentally
/// be included in what it covers, and so a file this device did not write —
/// one with no checksum at all — is rejected rather than half-trusted.
#[derive(Debug, Deserialize, Serialize)]
struct StateFile {
    /// Checksum over the canonical encoding of `state`.
    checksum: String,
    /// The state itself.
    state: PersistentState,
}

/// Encodes a state file, checksum included.
///
/// Public so a test or a scenario can seed a device with a state file the
/// device will accept, without hand-writing an envelope that would then have to
/// be kept in step with this one.
///
/// # Errors
///
/// Returns the encoding failure.
pub fn encode_state(state: &PersistentState) -> Result<Vec<u8>, serde_json::Error> {
    let file = StateFile {
        checksum: checksum_of(state),
        state: state.clone(),
    };
    serde_json::to_vec_pretty(&file)
}

/// Why the state file could not be persisted.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// The file could not be written.
    #[error("could not write the state file {path}: {source}")]
    Write {
        /// The path attempted.
        path: PathBuf,
        /// The underlying failure.
        source: std::io::Error,
    },
    /// The state could not be encoded.
    #[error("could not encode the state file: {0}")]
    Encode(#[from] serde_json::Error),
}

/// The state file and the state it holds.
#[derive(Debug)]
pub struct StateStore {
    path: PathBuf,
    state: PersistentState,
}

impl StateStore {
    /// Loads the state file, or fails closed if it cannot be trusted.
    ///
    /// Never returns an error: a device with unreadable state still boots, still
    /// senses, and still reports — it simply refuses to actuate. Refusing to
    /// start at all would turn a recoverable fault into a dead plant.
    #[must_use]
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let state = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<StateFile>(&bytes) {
                Ok(file) if checksum_of(&file.state) == file.checksum => file.state,
                Ok(file) => {
                    let kept = quarantine(&path);
                    tracing::error!(
                        path = %path.display(),
                        stored = %file.checksum,
                        computed = %checksum_of(&file.state),
                        kept = %kept.display(),
                        "persisted state failed its checksum; actuation is disabled"
                    );
                    PersistentState::failed_closed(PersistentStateFault {
                        reason: PersistentStateFault::CHECKSUM.to_owned(),
                        detail: format!(
                            "stored {} but computed {}; the unreadable file was kept at {}",
                            file.checksum,
                            checksum_of(&file.state),
                            kept.display()
                        ),
                    })
                }
                Err(e) => {
                    let kept = quarantine(&path);
                    tracing::error!(
                        path = %path.display(),
                        error = %e,
                        kept = %kept.display(),
                        "persisted state is corrupt; actuation is disabled until it is recovered"
                    );
                    PersistentState::failed_closed(PersistentStateFault {
                        reason: PersistentStateFault::CORRUPT.to_owned(),
                        detail: format!("{e}; the unreadable file was kept at {}", kept.display()),
                    })
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // A first boot is not a fault: there is nothing to distrust.
                tracing::info!(path = %path.display(), "no persisted state; first boot");
                PersistentState::default()
            }
            Err(e) => {
                tracing::error!(
                    path = %path.display(),
                    error = %e,
                    "persisted state could not be read; actuation is disabled"
                );
                PersistentState::failed_closed(PersistentStateFault {
                    reason: PersistentStateFault::UNREADABLE.to_owned(),
                    detail: e.to_string(),
                })
            }
        };
        let mut store = Self { path, state };
        store.state.boot_count = store.state.boot_count.saturating_add(1);
        // Persist the incremented boot count — and, after a corrupt load, the
        // fault itself — so the lockout survives the next restart too.
        if let Err(e) = store.save() {
            tracing::error!(error = %e, "could not record the boot");
        }
        store
    }

    /// The path in use.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The current state.
    #[must_use]
    pub const fn state(&self) -> &PersistentState {
        &self.state
    }

    /// Whether actuation is permitted.
    #[must_use]
    pub const fn actuation_permitted(&self) -> bool {
        self.state.actuation_permitted()
    }

    /// The persistent-state fault, if any.
    #[must_use]
    pub const fn fault(&self) -> Option<&PersistentStateFault> {
        self.state.persistent_state_fault.as_ref()
    }

    /// Changes the state and writes it out.
    ///
    /// Every mutation goes through here so "written on every change" is a
    /// property of the type rather than a thing each call site remembers.
    ///
    /// # Errors
    ///
    /// Returns the write failure. The in-memory change is kept: losing it as
    /// well as the write would leave the process disagreeing with itself.
    pub fn mutate<T>(
        &mut self,
        change: impl FnOnce(&mut PersistentState) -> T,
    ) -> Result<T, StateError> {
        let outcome = change(&mut self.state);
        self.save()?;
        Ok(outcome)
    }

    /// Re-reads the state file from disk, without touching this store.
    ///
    /// The read-back half of ADR-015 §7 step 4. Verifying the copy still held
    /// in memory would prove only that the struct is intact; it would say
    /// nothing about what actually reached storage, which is the thing that has
    /// to survive a power cut.
    ///
    /// # Errors
    ///
    /// Returns the read or decode failure.
    pub fn read_back(&self) -> Result<PersistentState, StateError> {
        let bytes = std::fs::read(&self.path).map_err(|source| StateError::Write {
            path: self.path.clone(),
            source,
        })?;
        let file: StateFile = serde_json::from_slice(&bytes)?;
        if checksum_of(&file.state) != file.checksum {
            return Err(StateError::Write {
                path: self.path.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "the state file failed its checksum on read-back",
                ),
            });
        }
        Ok(file.state)
    }

    /// Writes the state atomically: temp file, flush, rename.
    ///
    /// # Errors
    ///
    /// Returns the write failure.
    pub fn save(&self) -> Result<(), StateError> {
        let encoded = encode_state(&self.state)?;
        let temp = self.path.with_extension("json.tmp");
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|source| StateError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let write = || -> std::io::Result<()> {
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(&encoded)?;
            // Flush before the rename, or the rename can publish a file whose
            // contents are still in the page cache.
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temp, &self.path)
        };
        write().map_err(|source| StateError::Write {
            path: self.path.clone(),
            source,
        })
    }
}

/// Moves an untrusted state file aside, returning where it went.
///
/// Kept rather than deleted: the contents are the only evidence of what went
/// wrong, and overwriting them is how a corruption bug becomes unreproducible.
fn quarantine(path: &Path) -> PathBuf {
    let kept = path.with_extension("json.corrupt");
    if let Err(e) = std::fs::rename(path, &kept) {
        tracing::warn!(error = %e, "could not set the corrupt state file aside");
        return path.to_path_buf();
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhizo_mqtt_contract::payload::{CommandOrigin, CommandStatus};
    use uuid::Uuid;

    /// A unique scratch path per test, cleaned up on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "rhizo-state-{name}-{}-{:?}.json",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("json.corrupt"));
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(self.0.with_extension("json.corrupt"));
            let _ = std::fs::remove_file(self.0.with_extension("json.tmp"));
        }
    }

    fn command_id(n: u128) -> CommandId {
        CommandId::from_uuid(Uuid::from_u128(n))
    }

    fn record(n: u128) -> CommandRecord {
        CommandRecord {
            command_id: command_id(n),
            result: CommandResult {
                command_id: command_id(n),
                status: CommandStatus::Completed,
                requested_ml: 40.0,
                delivered_ml: Some(40.0),
                duration_ms: Some(4_878),
                clamped: false,
                reason: None,
                delivered_today_ml: 40.0,
                origin: CommandOrigin::EdgeCommand,
                detail: None,
            },
        }
    }

    // ------------------------------------------------------------ round trip

    #[test]
    fn state_survives_a_restart() {
        let scratch = Scratch::new("roundtrip");
        {
            let mut store = StateStore::load(scratch.path());
            assert_eq!(store.state().boot_count, 1);
            store
                .mutate(|s| {
                    s.applied_config_version = Some(7);
                    s.delivered_today_ml = 130.0;
                    s.record_command(record(1));
                    s.offline_runtime.cooldown_remaining_ms = 14_400_000;
                })
                .unwrap();
        }
        let store = StateStore::load(scratch.path());
        assert_eq!(store.state().boot_count, 2, "the boot count advances");
        assert_eq!(store.state().applied_config_version, Some(7));
        assert_eq!(store.state().delivered_today_ml, 130.0);
        assert_eq!(
            store.state().offline_runtime.cooldown_remaining_ms,
            14_400_000
        );
        assert!(store.state().previous(command_id(1)).is_some());
        assert!(store.actuation_permitted());
    }

    #[test]
    fn the_command_ring_deduplicates_across_restarts() {
        let scratch = Scratch::new("dedup-restart");
        {
            let mut store = StateStore::load(scratch.path());
            store.mutate(|s| s.record_command(record(42))).unwrap();
        }
        let store = StateStore::load(scratch.path());
        let previous = store
            .state()
            .previous(command_id(42))
            .expect("the ring must survive the restart");
        assert_eq!(previous.result.status, CommandStatus::Completed);
        assert_eq!(previous.result.delivered_ml, Some(40.0));
    }

    #[test]
    fn the_ring_evicts_at_sixteen_and_keeps_the_newest() {
        let mut state = PersistentState::default();
        for n in 0..40 {
            state.record_command(record(n));
        }
        assert_eq!(state.command_ring.len(), COMMAND_DEDUP_RING);
        assert!(state.previous(command_id(39)).is_some());
        assert!(state.previous(command_id(24)).is_some());
        assert!(
            state.previous(command_id(23)).is_none(),
            "the oldest entries are evicted"
        );
    }

    #[test]
    fn re_recording_a_command_updates_it_in_place() {
        let mut state = PersistentState::default();
        state.record_command(record(1));
        let mut updated = record(1);
        updated.result.status = CommandStatus::Interrupted;
        updated.result.delivered_ml = None;
        state.record_command(updated);
        assert_eq!(state.command_ring.len(), 1, "not a second entry");
        assert_eq!(
            state.previous(command_id(1)).unwrap().result.status,
            CommandStatus::Interrupted
        );
    }

    // --------------------------------------------------------- daily rollover

    #[test]
    fn the_daily_total_resets_on_a_day_boundary() {
        let mut state = PersistentState::default();
        let day_one = UtcMillis(20_325 * MS_PER_DAY + 3_600_000);
        state.credit_delivery(130.0, Some(day_one));
        assert_eq!(state.delivered_today_ml, 130.0);
        state.credit_delivery(40.0, Some(UtcMillis(day_one.0 + 1_000)));
        assert_eq!(state.delivered_today_ml, 170.0);

        let day_two = UtcMillis(20_326 * MS_PER_DAY + 60_000);
        state.credit_delivery(10.0, Some(day_two));
        assert_eq!(state.delivered_today_ml, 10.0, "a new day starts fresh");
        assert_eq!(state.delivered_day_epoch, Some(20_326));
    }

    /// SAFETY-015: a device that cannot prove a day passed must not be handed a
    /// fresh allowance.
    #[test]
    fn an_unsynchronised_device_never_rolls_the_day_over() {
        let mut state = PersistentState::default();
        state.credit_delivery(400.0, Some(UtcMillis(20_325 * MS_PER_DAY)));
        for _ in 0..100 {
            state.credit_delivery(0.0, None);
        }
        assert_eq!(state.delivered_today_ml, 400.0);
        assert_eq!(state.delivered_day_epoch, Some(20_325));
    }

    #[test]
    fn a_clock_that_steps_backwards_never_replenishes_the_budget() {
        let mut state = PersistentState::default();
        state.credit_delivery(400.0, Some(UtcMillis(20_325 * MS_PER_DAY)));
        state.roll_day(Some(UtcMillis(20_000 * MS_PER_DAY)));
        assert_eq!(
            state.delivered_today_ml, 400.0,
            "a backwards clock is not a new day"
        );
    }

    // -------------------------------------------------------------- fail closed

    #[test]
    fn a_corrupt_state_file_disables_actuation_and_reports_a_fault() {
        let scratch = Scratch::new("corrupt");
        std::fs::write(scratch.path(), b"{ this is not json").unwrap();

        let store = StateStore::load(scratch.path());
        assert!(
            !store.actuation_permitted(),
            "corruption must never restore actuation permission"
        );
        let fault = store.fault().expect("the fault must be observable");
        assert_eq!(fault.reason, PersistentStateFault::CORRUPT);
        assert!(
            scratch.path().with_extension("json.corrupt").exists(),
            "the evidence is kept, not overwritten"
        );
    }

    #[test]
    fn a_corrupt_state_file_never_becomes_a_permissive_fresh_start() {
        let scratch = Scratch::new("corrupt-permissive");
        std::fs::write(scratch.path(), b"\x00\x01\x02 truncated").unwrap();
        let store = StateStore::load(scratch.path());
        let state = store.state();

        assert_eq!(
            state.delivered_today_ml, FIRMWARE_MAX_DAILY_ML,
            "the budget is spent, never replenished"
        );
        assert_eq!(
            state.offline_runtime.cooldown_remaining_ms,
            u64::MAX,
            "the cooldown is as long as it can be, never shortened"
        );
        assert_eq!(
            state.offline_runtime.budget_window.delivered_ml,
            FIRMWARE_MAX_DAILY_ML
        );
        assert_eq!(state.offline_runtime.dose_count, u16::MAX);
        assert!(
            state.policy_active.is_none(),
            "no policy is activated and no default substituted"
        );
        assert!(state.policy_staging.is_none());
        assert!(state.applied_policy_versions.is_empty());
        assert!(!state.actuation_permitted());
    }

    #[test]
    fn the_lockout_survives_the_next_restart() {
        let scratch = Scratch::new("corrupt-persists");
        std::fs::write(scratch.path(), b"not json at all").unwrap();
        {
            let store = StateStore::load(scratch.path());
            assert!(!store.actuation_permitted());
        }
        // The second boot reads a perfectly valid file — one that records the
        // fault. A lockout that evaporated on the next restart would be no
        // lockout at all.
        let store = StateStore::load(scratch.path());
        assert!(!store.actuation_permitted());
        assert_eq!(store.fault().unwrap().reason, PersistentStateFault::CORRUPT);
        assert_eq!(store.state().boot_count, 2);
    }

    #[test]
    fn a_missing_state_file_is_a_first_boot_not_a_fault() {
        let scratch = Scratch::new("missing");
        let store = StateStore::load(scratch.path());
        assert!(store.actuation_permitted());
        assert!(store.fault().is_none());
        assert_eq!(store.state().boot_count, 1);
        assert_eq!(store.state().delivered_today_ml, 0.0);
    }

    // ------------------------------------------------------------- atomicity

    #[test]
    fn a_write_leaves_no_partial_file_behind() {
        let scratch = Scratch::new("atomic");
        let mut store = StateStore::load(scratch.path());
        for n in 0..50 {
            store.mutate(|s| s.record_command(record(n))).unwrap();
            // At every point between writes the file on disk must be a valid
            // whole state, never a truncated one.
            let bytes = std::fs::read(scratch.path()).unwrap();
            serde_json::from_slice::<PersistentState>(&bytes)
                .expect("the file on disk is always complete");
            assert!(
                !scratch.path().with_extension("json.tmp").exists(),
                "the temporary file is renamed away, not left behind"
            );
        }
    }

    #[test]
    fn a_state_file_written_by_a_kill_mid_write_is_detected_not_trusted() {
        // The simulation of a kill: the temp file exists with partial contents
        // while the real file still holds the last complete state. That is what
        // the temp-then-rename discipline guarantees.
        let scratch = Scratch::new("kill-mid-write");
        let mut store = StateStore::load(scratch.path());
        store.mutate(|s| s.delivered_today_ml = 42.0).unwrap();
        std::fs::write(scratch.path().with_extension("json.tmp"), b"{partial").unwrap();

        let reloaded = StateStore::load(scratch.path());
        assert!(reloaded.actuation_permitted());
        assert_eq!(reloaded.state().delivered_today_ml, 42.0);
        let _ = std::fs::remove_file(scratch.path().with_extension("json.tmp"));
    }

    // -------------------------------------------------------------- checksums

    #[test]
    fn a_stored_policy_verifies_against_its_checksum_and_a_tampered_one_does_not() {
        let stored = StoredPolicy::new(
            OfflinePolicySet {
                policies: Vec::new(),
            },
            BTreeMap::new(),
        );
        assert!(stored.verify());
        let mut tampered = stored.clone();
        tampered.checksum = String::from("crc32:deadbeef");
        assert!(
            !tampered.verify(),
            "a blob that fails read-back is refused, with no default substituted"
        );
    }

    #[test]
    fn the_checksum_is_stable_and_distinguishes_content() {
        let empty = checksum_of(&serde_json::json!({}));
        assert_eq!(empty, checksum_of(&serde_json::json!({})));
        assert_ne!(empty, checksum_of(&serde_json::json!({ "a": 1 })));
        assert!(empty.starts_with("crc32:"));
    }

    #[test]
    fn the_crc_matches_the_published_check_value() {
        // The IEEE CRC-32 of "123456789" is 0xCBF43926, the standard check
        // value. Pinning it means a future rewrite of the loop is verifiable
        // rather than merely self-consistent.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    // ------------------------------------------------- the whole inventory

    #[test]
    fn every_field_of_the_documented_inventory_round_trips() {
        let scratch = Scratch::new("inventory");
        let mut store = StateStore::load(scratch.path());
        store
            .mutate(|s| {
                s.applied_config_version = Some(7);
                s.delivered_today_ml = 130.0;
                s.delivered_day_epoch = Some(20_325);
                s.record_command(record(1));
                s.in_flight_dose = Some(InFlightDose {
                    command_id: command_id(2),
                    started_at_ms: Some(UtcMillis(1_756_121_500_000)),
                    started_at_monotonic_ms: 912_344,
                    requested_ml: 40.0,
                    effective_ml: 40.0,
                });
                s.pending_results.push(record(3).result);
                s.policy_active = Some(StoredPolicy::new(
                    OfflinePolicySet {
                        policies: Vec::new(),
                    },
                    BTreeMap::from([(String::from("monstera-01"), 7)]),
                ));
                s.applied_policy_versions
                    .insert(String::from("monstera-01"), 7);
                s.offline_runtime = OfflineRuntime {
                    cycle: crate::offline_state::CyclePhase::Cooldown,
                    budget_window: BudgetWindow {
                        elapsed_ms: 1_800_000,
                        delivered_ml: 70.0,
                    },
                    cooldown_remaining_ms: 14_400_000,
                    confirmation_elapsed_ms: 45_000,
                    dose_count: 2,
                };
                s.offline_events.next_seq = 4_381;
                s.offline_events.gap = Some(GapMetadata {
                    event_id: rhizo_mqtt_contract::EventId::from_uuid(Uuid::from_u128(77)),
                    device_seq: 4_381,
                    monotonic_ms: 8_814_000,
                    device_time_ms: None,
                    from_seq: 4_100,
                    to_seq: 4_380,
                    lost_count: 281,
                    lost_tier: EventTier::Telemetry,
                });
            })
            .unwrap();
        let before = store.state().clone();
        drop(store);

        let reloaded = StateStore::load(scratch.path());
        let after = reloaded.state();
        assert_eq!(after.boot_count, before.boot_count + 1);
        assert_eq!(after.in_flight_dose, before.in_flight_dose);
        assert_eq!(after.pending_results, before.pending_results);
        assert_eq!(after.policy_active, before.policy_active);
        assert_eq!(
            after.applied_policy_versions,
            before.applied_policy_versions
        );
        assert_eq!(after.offline_runtime, before.offline_runtime);
        assert_eq!(after.offline_events, before.offline_events);
        assert!(
            after.policy_active.as_ref().unwrap().verify(),
            "the stored checksum still matches after a round trip"
        );
    }

    #[test]
    fn an_unknown_field_in_a_stored_file_is_ignored_rather_than_fatal() {
        let scratch = Scratch::new("forward-compatible");
        // A file written by a future build: an extra field inside `state`.
        //
        // The checksum covers the *decoded* state, not the raw bytes, and that
        // distinction is deliberate. It means a field this build does not know
        // about is transparently ignored — protocol §9's forward-compatibility
        // rule applied to storage — while any change to a value this build
        // *does* act on still fails the check.
        let state = PersistentState {
            boot_count: 4,
            ..PersistentState::default()
        };
        let mut value: serde_json::Value =
            serde_json::from_slice(&encode_state(&state).unwrap()).unwrap();
        value["state"]["a_future_field"] = serde_json::json!({ "nested": true });
        std::fs::write(scratch.path(), serde_json::to_vec(&value).unwrap()).unwrap();

        let store = StateStore::load(scratch.path());
        assert!(
            store.actuation_permitted(),
            "a field this build does not know is not corruption"
        );
        assert_eq!(store.state().boot_count, 5);
    }

    #[test]
    fn a_single_flipped_digit_is_caught_by_the_checksum() {
        let scratch = Scratch::new("flipped-digit");
        {
            let mut store = StateStore::load(scratch.path());
            store.mutate(|s| s.delivered_today_ml = 460.0).unwrap();
        }
        // Still perfectly valid JSON, and four hundred millilitres more
        // permissive. Without a whole-file checksum this would be applied
        // silently.
        let text = std::fs::read_to_string(scratch.path()).unwrap();
        let damaged = text.replace("460.0", "60.0");
        assert_ne!(text, damaged, "the test must actually change something");
        std::fs::write(scratch.path(), damaged).unwrap();

        let store = StateStore::load(scratch.path());
        assert!(!store.actuation_permitted());
        assert_eq!(
            store.fault().unwrap().reason,
            PersistentStateFault::CHECKSUM
        );
        assert_eq!(
            store.state().delivered_today_ml,
            FIRMWARE_MAX_DAILY_ML,
            "and the budget is treated as spent, not as the damaged value"
        );
    }

    #[test]
    fn a_file_with_no_checksum_at_all_is_not_trusted() {
        let scratch = Scratch::new("no-checksum");
        std::fs::write(scratch.path(), br#"{"boot_count": 4}"#).unwrap();
        let store = StateStore::load(scratch.path());
        assert!(
            !store.actuation_permitted(),
            "a file this device did not write is not state it may act on"
        );
    }
}
