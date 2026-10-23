-- Identity of an imported memory (api/src/profile): equal keys are the same fact, whichever
-- source it came from, so re-imports and overlapping sources add no duplicates. NULL for
-- memories without one (manual, runs).
ALTER TABLE memories ADD COLUMN source_key TEXT;
CREATE UNIQUE INDEX memories_source_key_idx ON memories (source_key) WHERE source_key IS NOT NULL;
