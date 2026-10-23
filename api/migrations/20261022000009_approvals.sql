-- Guardrails for agent runs (see docs/agent-runtime.md). An `external` tool call waits in
-- `pending_actions` for the user's decision, and its step and run park as
-- `awaiting_approval`; a rejected action ends the run `rejected`. Every succeeded `write`
-- tool call is logged in `agent_actions` with what its undo needs. Low-confidence results
-- go to `review_items` instead of being applied. `paused` is the kill switch's
-- (20261022000008_kill_switch.sql).
ALTER TABLE agent_runs
    DROP CONSTRAINT agent_runs_status_check,
    ADD CONSTRAINT agent_runs_status_check
        CHECK (status IN ('queued','running','paused','succeeded','failed','awaiting_approval','rejected'));
ALTER TABLE run_steps
    DROP CONSTRAINT run_steps_status_check,
    ADD CONSTRAINT run_steps_status_check
        CHECK (status IN ('pending','running','succeeded','failed','awaiting_approval'));
ALTER TABLE tool_calls
    DROP CONSTRAINT tool_calls_status_check,
    ADD CONSTRAINT tool_calls_status_check
        CHECK (status IN ('running','succeeded','failed','invalid_input','invalid_output','unknown_tool','awaiting_approval'));

-- One row per distinct `external` request in a step. `input` is what the caller asked for
-- and keys the row with the step and tool, so a rerun of the step finds it again (jsonb's
-- text form is canonical). `approved_input` is what the user approved, possibly edited.
-- `tool_call_id` is the call that created the row; `executed_tool_call_id` the one that ran
-- the handler, whose `output` is kept so a rerun returns it without running it twice.
CREATE TABLE pending_actions (
    id BIGSERIAL PRIMARY KEY,
    run_id BIGINT NOT NULL REFERENCES agent_runs (id) ON DELETE CASCADE,
    run_step_id BIGINT NOT NULL REFERENCES run_steps (id) ON DELETE CASCADE,
    tool_call_id BIGINT NOT NULL REFERENCES tool_calls (id) ON DELETE CASCADE,
    tool TEXT NOT NULL,
    input JSONB NOT NULL,
    approved_input JSONB,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','approved','rejected','executed')),
    output JSONB,
    executed_tool_call_id BIGINT REFERENCES tool_calls (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at TIMESTAMPTZ,
    executed_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX pending_actions_key ON pending_actions (run_step_id, tool, md5(input::text));
-- `GET /pending-actions` reads the pending rows oldest first.
CREATE INDEX pending_actions_status_idx ON pending_actions (status, id);

-- One row per succeeded `write` tool call. `undo_data` is what the tool's undo needs (NULL
-- when the handler didn't set it, which makes the action impossible to undo); `undone_at`
-- is set once it has been undone.
CREATE TABLE agent_actions (
    id BIGSERIAL PRIMARY KEY,
    run_id BIGINT NOT NULL REFERENCES agent_runs (id) ON DELETE CASCADE,
    run_step_id BIGINT NOT NULL REFERENCES run_steps (id) ON DELETE CASCADE,
    tool_call_id BIGINT NOT NULL UNIQUE REFERENCES tool_calls (id) ON DELETE CASCADE,
    tool TEXT NOT NULL,
    undo_data JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    undone_at TIMESTAMPTZ
);
CREATE INDEX agent_actions_run_id_idx ON agent_actions (run_id, id);

-- A result a step was not sure enough of to apply. Keyed on the step, kind and payload so a
-- rerun of the step finds its item again.
CREATE TABLE review_items (
    id BIGSERIAL PRIMARY KEY,
    run_id BIGINT NOT NULL REFERENCES agent_runs (id) ON DELETE CASCADE,
    run_step_id BIGINT NOT NULL REFERENCES run_steps (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    confidence DOUBLE PRECISION NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    payload JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','accepted','dismissed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX review_items_key ON review_items (run_step_id, kind, md5(payload::text));
CREATE INDEX review_items_status_idx ON review_items (status, id);
