-- M6 — irrigation control and safety.
--
-- Forward-only and additive. The M6 command lifecycle reuses `commands`,
-- `watering_events`, and `irrigation_state` exactly as ADR-004 defined them;
-- what is new here is the durable **intent** a sleeping device's dose waits in
-- (ADR-018 §3), the pre-dose baselines recovery and no-delivery detection
-- compare against, and the lockout audit fields SAFETY-003 requires.
--
-- `commands` deliberately gains **no** column. An intent is not a command: no
-- `command_id` exists until delivery, so there is still exactly one
-- persist-before-publish moment per command and a delivery retry still reuses
-- the id allocated at that moment (SAFETY-001, SAFETY-010).

-- Who cleared a lockout, and when. An explicit reset is an operator action and
-- the record of it is what makes SAFETY-003's "explicit" half auditable.
ALTER TABLE plants ADD COLUMN lockout_cleared_by TEXT;
ALTER TABLE plants ADD COLUMN lockout_cleared_at INTEGER;

-- A lockout held for a fixed period regardless of whether its condition still
-- holds. F-060-51's forward clock step is the only writer: `Uncertain` is
-- otherwise auto-clearing, and a clock step must hold the plant for one cooldown
-- even though the inputs look fine the instant afterwards.
ALTER TABLE plants ADD COLUMN lockout_until INTEGER;

-- The readings taken immediately before the current cycle's first dose.
-- Recovery is judged against these (F-060-32), and so is no-delivery detection
-- (F-060-33) — which needs a weight baseline as well as a moisture one.
ALTER TABLE irrigation_state ADD COLUMN pre_dose_vwc REAL;
ALTER TABLE irrigation_state ADD COLUMN pre_dose_grams REAL;

-- A dose an operator asked for while the device was asleep.
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
