-- M5-001. Deleting a plant must preserve its watering history.
--
-- `watering_events.plant_id` references `plants`, so a hard delete would either
-- be refused by the foreign key or would have to orphan the rows. Neither is
-- acceptable: the ledger is the record of what the machine did to a living
-- thing, and it outlives the row that pointed at it. A soft delete keeps both
-- the rows and their attribution, and is the option M5-001 names first.
ALTER TABLE plants ADD COLUMN deleted_at INTEGER;
CREATE INDEX idx_plants_live ON plants(deleted_at);
