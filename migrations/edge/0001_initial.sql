-- Canonical pre-release baseline for the Rhizo Edge SQLite schema.
-- This is the complete schema through M6. Once the first release or deployment
-- exists, this file is immutable; all later schema changes require a new
-- forward-only numbered migration.
--
-- Column order here is asserted by
-- `storage::migrate::tests::canonical_baseline_contains_the_final_schema`.
-- Nothing reads a row positionally, so a change is not a correctness bug — but
-- it is schema churn, and the test is where that conversation happens.

CREATE TABLE devices (
    device_id TEXT PRIMARY KEY, name TEXT, firmware_version TEXT, boot_id TEXT,
    last_sequence INTEGER, status TEXT NOT NULL DEFAULT 'unknown',
    clock_synced INTEGER NOT NULL DEFAULT 0, last_seen_at INTEGER,
    desired_config_version INTEGER NOT NULL DEFAULT 0, applied_config_version INTEGER,
    created_at INTEGER NOT NULL, status_json TEXT, status_boot_generation INTEGER,
    status_sequence INTEGER, status_lwt_message_id TEXT, protocol_version INTEGER,
    uptime_ms INTEGER, free_heap_bytes INTEGER, rssi_dbm INTEGER,
    sensors_json TEXT NOT NULL DEFAULT '[]',
    telemetry_interval_seconds INTEGER NOT NULL DEFAULT 300, drift_since INTEGER,
    connectivity_mode TEXT NOT NULL DEFAULT 'connected', isolation_started_at INTEGER,
    last_time_sync_at INTEGER, power_mode TEXT NOT NULL DEFAULT 'always_on',
    wake_interval_seconds INTEGER, sleep_received_at INTEGER, expected_wake_at INTEGER,
    overdue_at INTEGER, missed_wake_count INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE plant_profiles (profile_id TEXT PRIMARY KEY, name TEXT NOT NULL, profile_json TEXT NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE plants (
    plant_id TEXT PRIMARY KEY, profile_id TEXT REFERENCES plant_profiles(profile_id),
    name TEXT NOT NULL, species TEXT, pot_volume_ml REAL, soil_type TEXT,
    auto_watering_enabled INTEGER NOT NULL DEFAULT 0, lockout_reason TEXT,
    lockout_since INTEGER, created_at INTEGER NOT NULL, deleted_at INTEGER,
    applied_preset_id TEXT, applied_catalogue_version INTEGER,
    -- Who cleared a lockout, and when. An explicit reset is an operator action
    -- and the record of it is what makes SAFETY-003's "explicit" half auditable.
    lockout_cleared_by TEXT, lockout_cleared_at INTEGER,
    -- A lockout held for a fixed period regardless of whether its condition
    -- still holds. F-060-51's forward clock step is the only writer:
    -- `Uncertain` is otherwise auto-clearing, and a clock step must hold the
    -- plant for one cooldown even though the inputs look fine the instant
    -- afterwards.
    lockout_until INTEGER
);
CREATE TABLE processed_messages (message_id TEXT PRIMARY KEY, device_id TEXT NOT NULL, kind TEXT NOT NULL, received_at INTEGER NOT NULL);
CREATE INDEX idx_processed_received ON processed_messages(received_at);
CREATE TABLE measurements (
    id INTEGER PRIMARY KEY AUTOINCREMENT, device_id TEXT NOT NULL REFERENCES devices(device_id),
    sensor_id TEXT, point TEXT NOT NULL DEFAULT 'default', kind TEXT NOT NULL,
    value_num REAL, value_bool INTEGER, unit TEXT NOT NULL, quality TEXT NOT NULL,
    calibration_ref TEXT, received_at INTEGER NOT NULL, device_time_ms INTEGER,
    boot_id TEXT, sequence INTEGER, batch_id TEXT NOT NULL,
    origin TEXT NOT NULL DEFAULT 'live', source_message_id TEXT, sample_index INTEGER
);
CREATE INDEX idx_meas_lookup ON measurements(device_id, point, kind, received_at DESC);
CREATE INDEX idx_meas_time ON measurements(received_at);
CREATE INDEX idx_meas_batch ON measurements(batch_id);
CREATE UNIQUE INDEX uq_measurement_batch_sample ON measurements(device_id, batch_id, sample_index) WHERE sample_index IS NOT NULL;
CREATE TABLE actuator_states (id INTEGER PRIMARY KEY AUTOINCREMENT, message_id TEXT NOT NULL UNIQUE, device_id TEXT NOT NULL, actuator_id TEXT NOT NULL, kind TEXT NOT NULL, state_json TEXT NOT NULL, received_at INTEGER NOT NULL, device_time_ms INTEGER, boot_id TEXT, sequence INTEGER);
CREATE TABLE command_results (message_id TEXT PRIMARY KEY, command_id TEXT NOT NULL, device_id TEXT NOT NULL, result_json TEXT NOT NULL, received_at INTEGER NOT NULL, device_time_ms INTEGER, boot_id TEXT, sequence INTEGER);
CREATE UNIQUE INDEX uq_command_result_command ON command_results(command_id);
CREATE TABLE device_events (event_id TEXT PRIMARY KEY, device_id TEXT NOT NULL, kind TEXT NOT NULL, severity TEXT NOT NULL, detail_json TEXT, occurred_at INTEGER NOT NULL, received_at INTEGER, boot_id TEXT, device_seq INTEGER, origin TEXT NOT NULL DEFAULT 'edge');
CREATE INDEX idx_devevents_device_time ON device_events(device_id, occurred_at DESC);
CREATE INDEX idx_devevents_replay ON device_events(device_id, boot_id, device_seq);
CREATE TABLE history_gaps (gap_id TEXT PRIMARY KEY, device_id TEXT NOT NULL, boot_id TEXT NOT NULL, from_seq INTEGER NOT NULL, to_seq INTEGER NOT NULL, lost_count INTEGER NOT NULL, tier TEXT NOT NULL, reported_at INTEGER NOT NULL);
-- `complete` records only the sender's final-batch marker. It is not proof of
-- a contiguous committed prefix and must never be used alone as reconciliation.
CREATE TABLE replay_progress (device_id TEXT NOT NULL, boot_id TEXT NOT NULL, through_device_seq INTEGER, complete INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL, PRIMARY KEY(device_id, boot_id));
CREATE TABLE quarantined_messages (id INTEGER PRIMARY KEY AUTOINCREMENT, device_id TEXT, topic TEXT NOT NULL, payload BLOB, error TEXT NOT NULL, received_at INTEGER NOT NULL);
CREATE TABLE commands (command_id TEXT PRIMARY KEY, device_id TEXT NOT NULL, plant_id TEXT REFERENCES plants(plant_id), kind TEXT NOT NULL, requested_ml REAL, mode TEXT NOT NULL, issued_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, status TEXT NOT NULL, published_at INTEGER, settled_at INTEGER, reason TEXT);
CREATE INDEX idx_commands_open ON commands(status, expires_at);
CREATE TABLE watering_events (watering_event_id TEXT PRIMARY KEY, plant_id TEXT REFERENCES plants(plant_id), device_id TEXT, command_id TEXT UNIQUE REFERENCES commands(command_id), mode TEXT NOT NULL, origin TEXT NOT NULL DEFAULT 'edge_command', started_at INTEGER NOT NULL, completed_at INTEGER, requested_ml REAL, delivered_ml REAL, status TEXT NOT NULL, reason_json TEXT);
CREATE INDEX idx_watering_plant_time ON watering_events(plant_id, completed_at DESC);
CREATE TABLE device_capabilities (device_id TEXT NOT NULL REFERENCES devices(device_id), capability_id TEXT NOT NULL, class TEXT NOT NULL, kinds_json TEXT NOT NULL, point TEXT, limits_json TEXT, declared_at INTEGER NOT NULL, PRIMARY KEY(device_id, capability_id));
CREATE TABLE sensor_bindings (binding_id TEXT PRIMARY KEY, plant_id TEXT NOT NULL REFERENCES plants(plant_id) ON DELETE CASCADE, device_id TEXT NOT NULL, sensor_id TEXT NOT NULL, point TEXT NOT NULL DEFAULT 'default', kind TEXT NOT NULL, role TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE INDEX idx_binding_plant ON sensor_bindings(plant_id, role);
CREATE UNIQUE INDEX uq_binding_control ON sensor_bindings(plant_id) WHERE role='control';
CREATE TABLE actuator_bindings (plant_id TEXT PRIMARY KEY REFERENCES plants(plant_id) ON DELETE CASCADE, device_id TEXT NOT NULL, actuator_id TEXT NOT NULL, kind TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE measurement_policies (plant_id TEXT NOT NULL REFERENCES plants(plant_id) ON DELETE CASCADE, kind TEXT NOT NULL, target_min REAL, target_max REAL, warning_low REAL, warning_high REAL, critical_low REAL, critical_high REAL, stale_after_ms INTEGER NOT NULL, hysteresis REAL, confirm_duration_ms INTEGER, updated_at INTEGER NOT NULL, PRIMARY KEY(plant_id,kind));
CREATE TABLE offline_policies (plant_id TEXT PRIMARY KEY REFERENCES plants(plant_id) ON DELETE CASCADE, policy_version INTEGER NOT NULL, enabled INTEGER NOT NULL DEFAULT 0, policy_json TEXT NOT NULL, published_at INTEGER, applied_version INTEGER, applied_at INTEGER, updated_at INTEGER NOT NULL);
-- `pre_dose_vwc` and `pre_dose_grams` are the readings taken immediately before
-- the current cycle's first dose: recovery is judged against these (F-060-32),
-- and so is no-delivery detection (F-060-33), which needs a weight baseline as
-- well as a moisture one.
CREATE TABLE irrigation_state (plant_id TEXT PRIMARY KEY REFERENCES plants(plant_id), state TEXT NOT NULL, state_since INTEGER NOT NULL, doses_this_cycle INTEGER NOT NULL DEFAULT 0, cycle_started_at INTEGER, last_cycle_completed_at INTEGER, wait_until INTEGER, active_command_id TEXT REFERENCES commands(command_id), updated_at INTEGER NOT NULL, pre_dose_vwc REAL, pre_dose_grams REAL);

-- A dose an operator asked for while the device was asleep.
--
-- `commands` deliberately has **no** matching column. An intent is not a command: no
-- `command_id` exists until delivery, so there is still exactly one
-- persist-before-publish moment per command and a delivery retry still reuses
-- the id allocated at that moment (SAFETY-001, SAFETY-010).
--
-- `command_id` is nullable and stays NULL until the wake that mints the real
-- command, which is the shape a reviewer checks this was implemented correctly.
-- `intent_expires_at` is the **edge's** clock and never reaches a device; the
-- wire TTL is unchanged at 120 s and is what the device validates (SAFETY-002).
CREATE TABLE command_intents (
    intent_id TEXT PRIMARY KEY,
    plant_id TEXT NOT NULL REFERENCES plants(plant_id),
    device_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    requested_ml REAL NOT NULL,
    mode TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    intent_expires_at INTEGER NOT NULL,
    expected_delivery_after INTEGER,
    state TEXT NOT NULL,
    command_id TEXT REFERENCES commands(command_id),
    refusal_reason TEXT,
    settled_at INTEGER,
    updated_at INTEGER NOT NULL
);

-- At most one open water intent per plant. The rolling cap would bound the total
-- anyway, but arriving at the cap by accident is not a design (ADR-018 §3).
CREATE UNIQUE INDEX uq_open_water_intent
    ON command_intents(plant_id)
    WHERE state = 'pending_for_device_wake' AND kind = 'water';
CREATE INDEX idx_intents_open ON command_intents(state, intent_expires_at);
CREATE TABLE pending_cloud_events (event_id TEXT PRIMARY KEY, kind TEXT NOT NULL, value_tier TEXT NOT NULL, payload_json TEXT NOT NULL, status TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, next_attempt_at INTEGER NOT NULL, last_error TEXT, created_at INTEGER NOT NULL, synced_at INTEGER);
CREATE INDEX idx_outbox_ready ON pending_cloud_events(status,next_attempt_at);

CREATE TABLE device_isolation_periods (id INTEGER PRIMARY KEY AUTOINCREMENT, device_id TEXT NOT NULL REFERENCES devices(device_id), started_at INTEGER NOT NULL, ended_at INTEGER, duration_ms INTEGER);
CREATE INDEX idx_device_isolation_periods ON device_isolation_periods(device_id, started_at DESC);
CREATE INDEX idx_devices_sleep_deadline ON devices(overdue_at) WHERE connectivity_mode = 'sleeping';
CREATE INDEX idx_plants_live ON plants(deleted_at);

CREATE TABLE plant_dry_state (plant_id TEXT PRIMARY KEY REFERENCES plants(plant_id) ON DELETE CASCADE, dry_ms INTEGER NOT NULL DEFAULT 0, last_sample_at INTEGER, updated_at INTEGER NOT NULL);
CREATE TABLE sensor_stuck_state (device_id TEXT NOT NULL, sensor_id TEXT NOT NULL, point TEXT NOT NULL, kind TEXT NOT NULL, last_bits INTEGER, last_bool INTEGER, last_received_at INTEGER, repeats INTEGER NOT NULL DEFAULT 0, reported INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL, PRIMARY KEY(device_id,sensor_id,point,kind));
CREATE TABLE plant_state_current (plant_id TEXT PRIMARY KEY REFERENCES plants(plant_id) ON DELETE CASCADE, state TEXT NOT NULL, since INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE plant_events (event_id TEXT PRIMARY KEY, plant_id TEXT, kind TEXT NOT NULL, severity TEXT NOT NULL, detail_json TEXT, occurred_at INTEGER NOT NULL);
CREATE INDEX idx_plant_events_time ON plant_events(plant_id, occurred_at DESC);
CREATE TABLE plant_recommendations (id INTEGER PRIMARY KEY AUTOINCREMENT, plant_id TEXT NOT NULL, decision TEXT NOT NULL, recommended_ml REAL, confidence REAL NOT NULL, reasons_json TEXT NOT NULL, blocked_by TEXT, evaluated_at INTEGER NOT NULL);
CREATE INDEX idx_reco_plant_time ON plant_recommendations(plant_id, evaluated_at DESC);
CREATE TABLE plant_threshold_state (plant_id TEXT NOT NULL REFERENCES plants(plant_id) ON DELETE CASCADE, kind TEXT NOT NULL, severity TEXT NOT NULL, candidate TEXT, candidate_since INTEGER, updated_at INTEGER NOT NULL, PRIMARY KEY(plant_id,kind));
