-- Which memories each run step retrieved (api/src/retrieval.rs). One row per step and
-- memory, with a snapshot of the memory's kind, topic and content at retrieval time, so
-- the run inspector shows what the run actually saw after the memory is edited or
-- deleted. Steps are at-least-once, so a rerun that retrieves the same memory adds no
-- second row (the unique index). Deleting a memory keeps the row with `memory_id`
-- cleared; deleting a run or step deletes its rows.
CREATE TABLE run_memories (
    id BIGSERIAL PRIMARY KEY,
    run_id BIGINT NOT NULL REFERENCES agent_runs (id) ON DELETE CASCADE,
    run_step_id BIGINT NOT NULL REFERENCES run_steps (id) ON DELETE CASCADE,
    memory_id BIGINT REFERENCES memories (id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK (kind IN ('fact','preference','answer')),
    topic TEXT NOT NULL,
    content TEXT NOT NULL,
    score DOUBLE PRECISION NOT NULL CHECK (score >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX run_memories_step_memory_idx ON run_memories (run_step_id, memory_id);
CREATE INDEX run_memories_run_id_idx ON run_memories (run_id, id);
