-- M5-018. Where a plant's starting numbers came from.
--
-- **Provenance only.** These columns are not a foreign key with behaviour and
-- nothing re-derives configuration from them: not on restart, not on a
-- catalogue upgrade, not on a tick. They exist so an operator can see where the
-- numbers came from, and so a later catalogue version can offer them a diff.
--
-- They are deliberately *not read* by recommendation, by the safety gate, by
-- irrigation control, or by offline-policy evaluation. Those four consume
-- measurement_policies, bindings, and measurements, exactly as they do for a
-- hand-configured plant, and cannot tell the two apart — which is the property
-- that makes a preset a starting point rather than a second configuration
-- authority.
ALTER TABLE plants ADD COLUMN applied_preset_id TEXT;
ALTER TABLE plants ADD COLUMN applied_catalogue_version INTEGER;
