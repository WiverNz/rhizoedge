//! Compile-time-only virtual actuation and redacted bench observations.
use rhizo_node_app::persist::PersistedState;
use rhizo_node_app::ports::{Pump, PumpError};

/// No hardware handle exists in this adapter. Count is local to a boot;
/// command/boot IDs and durable state snapshots disambiguate reboot evidence.
#[derive(Default)]
pub struct VirtualPump {
    count: u64,
}
impl Pump for VirtualPump {
    fn run_for(&mut self, ms: u32) -> Result<(), PumpError> {
        self.count += 1;
        log::warn!("VIRTUAL_EXEC count={} run_ms={ms}", self.count);
        Ok(())
    }
    fn off(&mut self) {}
    fn is_faulted(&self) -> bool {
        false
    }
}

/// Emits only accounting and result state, never provisioning secrets.
pub fn snapshot(label: &str, state: &PersistedState) {
    let value = serde_json::json!({
        "label": label, "generation": state.boot_generation,
        "daily": state.daily, "pending": state.pending_results.entries().iter().map(|e| e.result.command_id).collect::<Vec<_>>(),
        "unacknowledged_ml": state.pending_results.unacknowledged_ml(),
        "dedup": state.dedup_ring.iter().map(|e| e.command_id).collect::<Vec<_>>(), "in_flight": state.in_flight_dose,
        "offline": state.offline_runtime,
    });
    log::warn!("VIRTUAL_STATE {value}");
}
