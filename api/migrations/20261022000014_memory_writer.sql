-- The memory writer (api/src/workflows/memory_writer.rs, docs/memory-writer.md): a workflow
-- that reads unprocessed learning signals, asks Claude what durable facts, preferences or
-- answers they show, and stores the valid ones as `proposed` memories with their evidence.
--
-- A writer run claims a signal by setting `writer_run_id`, and marks it processed by setting
-- `processed_at` in the same transaction as the memories it stores. A claim only counts while
-- its run is `queued`, `running` or `paused`, so a run that fails for good (or is deleted)
-- releases its signals with no cleanup. `revises_memory_id` points a proposed revision at the
-- memory it would replace; the writer never overwrites a confirmed memory.
-- `memory_evidence` links a learned memory to the signals that support it; both foreign keys
-- cascade, so deleting either side drops the link.

ALTER TABLE memory_signals
    ADD COLUMN processed_at TIMESTAMPTZ,
    ADD COLUMN writer_run_id BIGINT REFERENCES agent_runs (id) ON DELETE SET NULL;

CREATE INDEX memory_signals_unprocessed_idx ON memory_signals (id) WHERE processed_at IS NULL;

ALTER TABLE memories
    DROP CONSTRAINT memories_source_kind_check,
    ADD CONSTRAINT memories_source_kind_check
        CHECK (source_kind IN ('manual','resume','linkedin','run','email','signal'));

ALTER TABLE memories ADD COLUMN revises_memory_id BIGINT REFERENCES memories (id) ON DELETE SET NULL;

CREATE TABLE memory_evidence (
    memory_id BIGINT NOT NULL REFERENCES memories (id) ON DELETE CASCADE,
    signal_id BIGINT NOT NULL REFERENCES memory_signals (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (memory_id, signal_id)
);

CREATE INDEX memory_evidence_signal_id_idx ON memory_evidence (signal_id);
