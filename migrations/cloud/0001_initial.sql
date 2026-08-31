CREATE TABLE edge_instances (
    edge_id TEXT PRIMARY KEY,
    display_name TEXT,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE synced_events (
    id BIGSERIAL PRIMARY KEY,
    edge_id TEXT NOT NULL REFERENCES edge_instances(edge_id),
    event_id UUID NOT NULL,
    kind TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    device_id TEXT,
    plant_id TEXT,
    payload JSONB NOT NULL,
    CONSTRAINT uq_synced_events UNIQUE (edge_id, event_id)
);
CREATE INDEX idx_synced_kind_time ON synced_events(edge_id, kind, occurred_at DESC);

CREATE TABLE devices (
    edge_id TEXT NOT NULL REFERENCES edge_instances(edge_id),
    device_id TEXT NOT NULL,
    name TEXT,
    firmware_version TEXT,
    status TEXT,
    last_seen_at TIMESTAMPTZ,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY(edge_id, device_id)
);
CREATE TABLE plants (
    edge_id TEXT NOT NULL REFERENCES edge_instances(edge_id),
    plant_id TEXT NOT NULL,
    name TEXT,
    species TEXT,
    bindings_json JSONB,
    policies_json JSONB,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at TIMESTAMPTZ,
    PRIMARY KEY(edge_id, plant_id)
);
CREATE TABLE measurements (
    edge_id TEXT NOT NULL REFERENCES edge_instances(edge_id),
    device_id TEXT NOT NULL,
    point TEXT NOT NULL DEFAULT 'default',
    kind TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    value_num DOUBLE PRECISION,
    value_bool BOOLEAN,
    unit TEXT NOT NULL,
    quality TEXT NOT NULL,
    sensor_id TEXT,
    calibration_ref TEXT,
    batch_id UUID,
    origin TEXT NOT NULL DEFAULT 'live',
    plant_id TEXT,
    PRIMARY KEY(edge_id, device_id, point, kind, occurred_at)
);
CREATE INDEX idx_cloud_meas_batch ON measurements(edge_id, batch_id);
CREATE INDEX idx_cloud_meas_plant_time ON measurements(edge_id, plant_id, occurred_at DESC);
CREATE TABLE watering_events (
    edge_id TEXT NOT NULL REFERENCES edge_instances(edge_id),
    watering_event_id UUID NOT NULL,
    plant_id TEXT NOT NULL,
    mode TEXT NOT NULL,
    origin TEXT NOT NULL DEFAULT 'edge_command',
    started_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    requested_ml DOUBLE PRECISION,
    delivered_ml DOUBLE PRECISION,
    status TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY(edge_id, watering_event_id)
);
CREATE INDEX idx_cloud_watering_plant_time ON watering_events(edge_id, plant_id, started_at DESC);
CREATE TABLE device_events (
    edge_id TEXT NOT NULL REFERENCES edge_instances(edge_id),
    event_id UUID NOT NULL,
    device_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL,
    detail JSONB,
    occurred_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY(edge_id, event_id)
);
