-- Status heartbeats use fresh transport message ids.  Keep only a bounded
-- logical high-water projection so transport dedup markers remain prunable.
ALTER TABLE devices ADD COLUMN status_boot_generation INTEGER;
ALTER TABLE devices ADD COLUMN status_sequence INTEGER;
ALTER TABLE devices ADD COLUMN status_lwt_message_id TEXT;
