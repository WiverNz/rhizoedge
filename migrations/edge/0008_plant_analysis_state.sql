-- M5-006, M5-008, M5-010, M5-012, M5-015.
--
-- Durable state for the analysis the plant tick performs. Each table exists
-- because losing its contents across a restart would either lose progress or
-- invent it, and inventing it is the dangerous direction.

-- M5-006. Continuous time below the target, and the receipt time it was last
-- advanced from. Persisted so a restart mid-debounce neither loses progress nor
-- fabricates the silence it slept through.
CREATE TABLE plant_dry_state (
  plant_id TEXT PRIMARY KEY REFERENCES plants(plant_id) ON DELETE CASCADE,
  dry_ms INTEGER NOT NULL DEFAULT 0,
  last_sample_at INTEGER,
  updated_at INTEGER NOT NULL
);

-- M5-008. Run length of bit-identical readings per sensor stream. `reported`
-- is what makes `sensor_stuck` one event per run rather than one per sample.
--
-- `last_received_at` records which reading the run last consumed. The tick reads
-- the *latest* row rather than a stream of new ones, so without it every tick
-- would fold the same reading in again and any sensor at all would look stuck
-- within ten minutes.
CREATE TABLE sensor_stuck_state (
  device_id TEXT NOT NULL,
  point TEXT NOT NULL,
  kind TEXT NOT NULL,
  last_bits INTEGER,
  last_bool INTEGER,
  last_received_at INTEGER,
  repeats INTEGER NOT NULL DEFAULT 0,
  reported INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (device_id, point, kind)
);

-- M5-010. The current operator-facing state. Transitions go to plant_events;
-- this row is only what the last evaluation concluded, so a tick that changes
-- nothing writes nothing new.
CREATE TABLE plant_state_current (
  plant_id TEXT PRIMARY KEY REFERENCES plants(plant_id) ON DELETE CASCADE,
  state TEXT NOT NULL,
  since INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

-- Plant-scoped events, deliberately separate from device_events: these are
-- statements about a plant, and they outlive the device that happened to be
-- bound to it. No foreign key, for the same reason watering_events keeps its
-- rows when a plant is removed.
CREATE TABLE plant_events (
  event_id TEXT PRIMARY KEY,
  plant_id TEXT,
  kind TEXT NOT NULL,
  severity TEXT NOT NULL,
  detail_json TEXT,
  occurred_at INTEGER NOT NULL
);
CREATE INDEX idx_plant_events_time ON plant_events(plant_id, occurred_at DESC);

-- M5-012. One row per *change* of decision or reason set, never one per tick:
-- a 30-second tick would otherwise write 2 880 rows per plant per day to record
-- that nothing happened.
CREATE TABLE plant_recommendations (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  plant_id TEXT NOT NULL,
  decision TEXT NOT NULL,
  recommended_ml REAL,
  confidence REAL NOT NULL,
  reasons_json TEXT NOT NULL,
  blocked_by TEXT,
  evaluated_at INTEGER NOT NULL
);
CREATE INDEX idx_reco_plant_time ON plant_recommendations(plant_id, evaluated_at DESC);

-- M5-015. Threshold severity per (plant, kind), with the candidate waiting out
-- its confirmation window.
CREATE TABLE plant_threshold_state (
  plant_id TEXT NOT NULL REFERENCES plants(plant_id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  severity TEXT NOT NULL,
  candidate TEXT,
  candidate_since INTEGER,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (plant_id, kind)
);
