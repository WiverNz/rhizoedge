ALTER TABLE devices ADD COLUMN power_mode TEXT NOT NULL DEFAULT 'always_on';
ALTER TABLE devices ADD COLUMN wake_interval_seconds INTEGER;
ALTER TABLE devices ADD COLUMN sleep_received_at INTEGER;
ALTER TABLE devices ADD COLUMN expected_wake_at INTEGER;
ALTER TABLE devices ADD COLUMN overdue_at INTEGER;
ALTER TABLE devices ADD COLUMN missed_wake_count INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_devices_sleep_deadline
    ON devices(overdue_at)
    WHERE connectivity_mode = 'sleeping';
