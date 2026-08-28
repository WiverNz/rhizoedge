-- `through_device_seq` was NOT NULL DEFAULT 0, which made 0 mean two different
-- things: "sequence 0 is committed" and "nothing contiguous is committed yet".
-- `device_seq` is zero-based, so both are reachable, and the edge cannot honour
-- protocol section 5.13's prefix rule while it cannot tell them apart.
--
-- NULL now means "no contiguous prefix committed", and the edge publishes no
-- acknowledgement at all in that state. Acknowledging 0 when nothing is
-- committed would tell a device to discard the one event the edge never got.
CREATE TABLE replay_progress_v2 (
    device_id TEXT NOT NULL,
    boot_id TEXT NOT NULL,
    through_device_seq INTEGER,
    complete INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (device_id, boot_id)
);

-- An existing 0 is only genuine progress if sequence 0 was actually committed
-- for that boot; otherwise it is the old default standing in for "nothing".
INSERT INTO replay_progress_v2 (device_id, boot_id, through_device_seq, complete, updated_at)
SELECT
    p.device_id,
    p.boot_id,
    CASE
        WHEN p.through_device_seq = 0
             AND NOT EXISTS (
                 SELECT 1 FROM device_events e
                 WHERE e.device_id = p.device_id
                   AND e.boot_id = p.boot_id
                   AND e.device_seq = 0
             )
        THEN NULL
        ELSE p.through_device_seq
    END,
    p.complete,
    p.updated_at
FROM replay_progress p;

DROP TABLE replay_progress;
ALTER TABLE replay_progress_v2 RENAME TO replay_progress;
