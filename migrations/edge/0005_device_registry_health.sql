ALTER TABLE devices ADD COLUMN protocol_version INTEGER;
ALTER TABLE devices ADD COLUMN uptime_ms INTEGER;
ALTER TABLE devices ADD COLUMN free_heap_bytes INTEGER;
ALTER TABLE devices ADD COLUMN rssi_dbm INTEGER;
ALTER TABLE devices ADD COLUMN sensors_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE devices ADD COLUMN telemetry_interval_seconds INTEGER NOT NULL DEFAULT 300;
ALTER TABLE devices ADD COLUMN drift_since INTEGER;
ALTER TABLE devices ADD COLUMN connectivity_mode TEXT NOT NULL DEFAULT 'connected';
ALTER TABLE devices ADD COLUMN isolation_started_at INTEGER;
ALTER TABLE devices ADD COLUMN last_time_sync_at INTEGER;

CREATE TABLE device_isolation_periods (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id TEXT NOT NULL REFERENCES devices(device_id),
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    duration_ms INTEGER
);
CREATE INDEX idx_device_isolation_periods ON device_isolation_periods(device_id, started_at DESC);
