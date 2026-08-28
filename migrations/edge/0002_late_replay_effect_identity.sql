-- Transport markers are retained for only seven days. Live telemetry therefore
-- needs a stable effect identity of its own, just like actuator states,
-- command results, and replayed events already have.
ALTER TABLE measurements ADD COLUMN source_message_id TEXT;
ALTER TABLE measurements ADD COLUMN sample_index INTEGER;

CREATE UNIQUE INDEX uq_measurement_batch_sample
    ON measurements(device_id, batch_id, sample_index)
    WHERE sample_index IS NOT NULL;

CREATE UNIQUE INDEX uq_command_result_command
    ON command_results(command_id);
