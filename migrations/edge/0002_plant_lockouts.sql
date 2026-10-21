-- Independent plant lockout records (stage 1 of the multi-lockout model).
--
-- `plants.lockout_reason` holds ONE reason, and every writer replaces it: a
-- clock-step hold or a `clock_unsynced` hold could overwrite a latched leak,
-- and a detected leak could overwrite a latched `no_delivery_detected`. This
-- table holds one row per independent reason, so no writer can displace
-- another's:
--
--   latch  explicit-clear (F-060-41). Removed only by an operator clearing
--          that one reason. `since` is the first time it was latched.
--   hold   a fixed-period block (F-060-51 clock step, `clock_unsynced`).
--          Expires by time only; never cleared by an operator.
--
-- Derived conditions (stale data, tank low, ...) are re-evaluated on every
-- pass and are never recorded here.
--
-- Nothing reads this table yet: until the cutover, `plants.lockout_reason`
-- stays authoritative and `repo::command::set_lockout` mirrors every write
-- into it, so the table can only ever hold *more* than the column.
CREATE TABLE plant_lockouts (
    plant_id TEXT NOT NULL REFERENCES plants(plant_id),
    reason TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('latch', 'hold')),
    since INTEGER NOT NULL,
    hold_until INTEGER,
    CHECK ((kind = 'hold') = (hold_until IS NOT NULL)),
    PRIMARY KEY (plant_id, reason)
);

-- The backfill, fail-closed. Every row of the single-reason column becomes a
-- record unless it is a *known* derived reason with no hold, which the next
-- pass re-derives from live inputs anyway:
--
--   * a known explicit-clear reason            -> latch
--   * an unrecognised or blank reason          -> latch (blank is named
--     'unknown'); an operator clears it by its exact stored name
--   * a known non-latch reason with a deadline -> hold, expired or not: the
--     reader compares against the edge clock, which this SQL cannot see
--
-- `since` falls back to the plant's creation time rather than dropping a row
-- whose timestamp is missing. Soft-deleted plants are migrated too; every
-- reader filters them. Idempotent by construction (ON CONFLICT DO NOTHING).
INSERT INTO plant_lockouts (plant_id, reason, kind, since, hold_until)
SELECT
    plant_id,
    CASE WHEN trim(lockout_reason) = '' THEN 'unknown' ELSE lockout_reason END,
    CASE
        WHEN lockout_reason IN ('uncertain', 'clock_unsynced', 'stale_data', 'sensor_fault',
                                'tank_low', 'daily_limit', 'no_actuator')
             AND lockout_until IS NOT NULL THEN 'hold'
        ELSE 'latch'
    END,
    coalesce(lockout_since, created_at),
    CASE
        WHEN lockout_reason IN ('uncertain', 'clock_unsynced', 'stale_data', 'sensor_fault',
                                'tank_low', 'daily_limit', 'no_actuator')
             THEN lockout_until
    END
FROM plants
WHERE lockout_reason IS NOT NULL
  AND NOT (lockout_reason IN ('uncertain', 'clock_unsynced', 'stale_data', 'sensor_fault',
                              'tank_low', 'daily_limit', 'no_actuator')
           AND lockout_until IS NULL)
ON CONFLICT (plant_id, reason) DO NOTHING;

-- One durable, local record per migrated row, so an operator can see what the
-- migration carried over and why — in particular an unrecognised reason, which
-- now blocks until someone clears it by name.
INSERT INTO plant_events (event_id, plant_id, kind, severity, detail_json, occurred_at)
SELECT
    'lockout_migrated:' || l.plant_id || ':' || l.reason,
    l.plant_id,
    'lockout_migrated',
    CASE
        WHEN l.reason IN ('leak', 'pump_fault', 'no_delivery_detected', 'max_doses_reached',
                          'uncertain', 'clock_unsynced', 'stale_data', 'sensor_fault',
                          'tank_low', 'daily_limit', 'no_actuator')
             THEN 'info'
        ELSE 'warning'
    END,
    json_object('reason', l.reason, 'kind', l.kind, 'since', l.since,
                'hold_until', l.hold_until, 'legacy_reason', p.lockout_reason),
    CAST(strftime('%s', 'now') AS INTEGER) * 1000
FROM plant_lockouts l
JOIN plants p ON p.plant_id = l.plant_id
-- SQLite needs a WHERE before an upsert clause on INSERT ... SELECT.
WHERE 1
ON CONFLICT (event_id) DO NOTHING;
