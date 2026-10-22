-- Agent runs (see docs/agent-runtime.md). One `agent_runs` row per run of a workflow, one
-- `run_steps` row per step of it, and one `tool_calls` row per tool invocation in a step.
-- Token and cost totals are sums of successful `claude -p` calls: a step's include its tool
-- calls', and a run's equal the sum of its steps'. Every UPDATE sets `updated_at = now()`
-- itself (there is no trigger).
CREATE TABLE agent_runs (
    id BIGSERIAL PRIMARY KEY,
    workflow TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','running','succeeded','failed')),
    input JSONB NOT NULL,
    output JSONB,
    error TEXT,
    input_tokens BIGINT NOT NULL DEFAULT 0 CHECK (input_tokens >= 0),
    output_tokens BIGINT NOT NULL DEFAULT 0 CHECK (output_tokens >= 0),
    cost_usd DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (cost_usd >= 0),
    -- The latest `agent_run` job for this run.
    job_id BIGINT REFERENCES jobs (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ
);

-- `attempts` counts every time the step started, across job retries and resumes.
-- `claude_call_id` is the step's most recent successful model call.
CREATE TABLE run_steps (
    id BIGSERIAL PRIMARY KEY,
    run_id BIGINT NOT NULL REFERENCES agent_runs (id) ON DELETE CASCADE,
    ordinal INT NOT NULL CHECK (ordinal >= 0),
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','running','succeeded','failed')),
    attempts INT NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    input JSONB,
    output JSONB,
    error TEXT,
    input_tokens BIGINT NOT NULL DEFAULT 0 CHECK (input_tokens >= 0),
    output_tokens BIGINT NOT NULL DEFAULT 0 CHECK (output_tokens >= 0),
    cost_usd DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (cost_usd >= 0),
    claude_call_id BIGINT REFERENCES claude_calls (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    UNIQUE (run_id, ordinal)
);

-- `permission` is NULL only for `unknown_tool`. `claude_call_id` is the model call that
-- asked for the tool, when there was one. Rows are written twice: `running` on insert,
-- then the final status.
CREATE TABLE tool_calls (
    id BIGSERIAL PRIMARY KEY,
    run_step_id BIGINT NOT NULL REFERENCES run_steps (id) ON DELETE CASCADE,
    tool TEXT NOT NULL,
    permission TEXT CHECK (permission IN ('read','write','external')),
    status TEXT NOT NULL CHECK (status IN ('running','succeeded','failed','invalid_input','invalid_output','unknown_tool')),
    input JSONB NOT NULL,
    output JSONB,
    error TEXT,
    input_tokens BIGINT NOT NULL DEFAULT 0 CHECK (input_tokens >= 0),
    output_tokens BIGINT NOT NULL DEFAULT 0 CHECK (output_tokens >= 0),
    cost_usd DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (cost_usd >= 0),
    claude_call_id BIGINT REFERENCES claude_calls (id) ON DELETE SET NULL,
    duration_ms BIGINT CHECK (duration_ms >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ
);

-- A run's detail reads each step's tool calls in order.
CREATE INDEX tool_calls_run_step_id_idx ON tool_calls (run_step_id, id);
