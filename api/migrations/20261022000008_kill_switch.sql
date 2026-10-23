-- The global kill switch (see docs/agent-runtime.md). One row; `id` is always TRUE.
CREATE TABLE kill_switch (
    id BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    paused BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO kill_switch DEFAULT VALUES;

-- A run waits as `paused` while the kill switch is on, and is `queued` again once it's off.
ALTER TABLE agent_runs DROP CONSTRAINT agent_runs_status_check;
ALTER TABLE agent_runs ADD CONSTRAINT agent_runs_status_check
    CHECK (status IN ('queued','running','paused','succeeded','failed'));
