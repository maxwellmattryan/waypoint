-- One row per unit of background work, claimed by the in-process worker. `run_at` is the
-- earliest time the job may start; retries and usage-limit deferrals push it forward.
-- Every UPDATE sets `updated_at = now()` itself (there is no trigger).
CREATE TABLE jobs (
    id BIGSERIAL PRIMARY KEY,
    kind TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','running','succeeded','failed')),
    attempts INT NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    max_attempts INT NOT NULL DEFAULT 3 CHECK (max_attempts >= 1),
    run_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error TEXT,
    result JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ
);

-- The worker's claim query: the oldest ready queued row.
CREATE INDEX jobs_ready_idx ON jobs (run_at, id) WHERE status = 'queued';
