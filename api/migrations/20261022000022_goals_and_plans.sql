-- Goals and plans (api/src/goals/, api/src/workflows/plan_goal.rs, docs/goals.md). A goal is
-- the user's request in their own words. The `plan_goal` workflow asks clarifying
-- questions (`goal_questions`) or writes a plan. Every plan write is a new version in
-- `plans`; the newest is the goal's `current_plan_id`. Each version has the same five
-- `plan_steps` (target_companies, source, score, shortlist, prep), run one at a time by
-- `plan_step` jobs. `plan_step_tasks` records the `source_board` jobs and `score_fit` runs
-- a step waits for, and `plan_shortlist` the postings the shortlist step picked. A step
-- copied from the previous version keeps a link in `kept_from`. Every UPDATE sets
-- `updated_at = now()` itself.

CREATE TABLE goals (
    id BIGSERIAL PRIMARY KEY,
    text TEXT NOT NULL,
    status TEXT NOT NULL
        CHECK (status IN ('planning','needs_answers','planned','running','completed','failed')),
    error TEXT,
    auto_replans INT NOT NULL DEFAULT 0 CHECK (auto_replans >= 0),
    current_plan_id BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX goals_status_idx ON goals (status);

CREATE TABLE goal_questions (
    id BIGSERIAL PRIMARY KEY,
    goal_id BIGINT NOT NULL REFERENCES goals (id) ON DELETE CASCADE,
    run_id BIGINT REFERENCES agent_runs (id) ON DELETE SET NULL,
    ordinal INT NOT NULL,
    question TEXT NOT NULL,
    answer TEXT,
    answered_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (goal_id, ordinal)
);

CREATE INDEX goal_questions_run_idx ON goal_questions (run_id);

CREATE TABLE plans (
    id BIGSERIAL PRIMARY KEY,
    goal_id BIGINT NOT NULL REFERENCES goals (id) ON DELETE CASCADE,
    version INT NOT NULL CHECK (version >= 1),
    author TEXT NOT NULL CHECK (author IN ('planner','user')),
    trigger TEXT NOT NULL CHECK (trigger IN ('goal','step_failed','user_replan','user_edit')),
    status TEXT NOT NULL CHECK (status IN ('ready','in_review','dismissed')),
    summary TEXT NOT NULL,
    confidence DOUBLE PRECISION,
    config JSONB NOT NULL,
    dropped JSONB NOT NULL DEFAULT '{}',
    run_id BIGINT UNIQUE REFERENCES agent_runs (id) ON DELETE SET NULL,
    review_item_id BIGINT UNIQUE REFERENCES review_items (id) ON DELETE SET NULL,
    claude_call_id BIGINT REFERENCES claude_calls (id) ON DELETE SET NULL,
    cost_usd DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (cost_usd >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (goal_id, version)
);

ALTER TABLE goals ADD CONSTRAINT goals_current_plan_fk
    FOREIGN KEY (current_plan_id) REFERENCES plans (id) ON DELETE SET NULL;

CREATE TABLE plan_steps (
    id BIGSERIAL PRIMARY KEY,
    plan_id BIGINT NOT NULL REFERENCES plans (id) ON DELETE CASCADE,
    ordinal INT NOT NULL,
    kind TEXT NOT NULL
        CHECK (kind IN ('target_companies','source','score','shortlist','prep')),
    status TEXT NOT NULL CHECK (status IN ('pending','running','succeeded','failed','skipped')),
    config JSONB NOT NULL,
    output JSONB,
    last_error TEXT,
    kept_from BIGINT REFERENCES plan_steps (id) ON DELETE SET NULL,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (plan_id, ordinal)
);

CREATE TABLE plan_step_tasks (
    id BIGSERIAL PRIMARY KEY,
    step_id BIGINT NOT NULL REFERENCES plan_steps (id) ON DELETE CASCADE,
    job_id BIGINT REFERENCES jobs (id) ON DELETE SET NULL,
    run_id BIGINT REFERENCES agent_runs (id) ON DELETE SET NULL,
    posting_id BIGINT REFERENCES job_postings (id) ON DELETE SET NULL,
    source TEXT,
    board TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (job_id IS NOT NULL OR run_id IS NOT NULL)
);

CREATE INDEX plan_step_tasks_step_idx ON plan_step_tasks (step_id);
CREATE INDEX plan_step_tasks_job_idx ON plan_step_tasks (job_id);
CREATE INDEX plan_step_tasks_run_idx ON plan_step_tasks (run_id);

CREATE TABLE plan_shortlist (
    step_id BIGINT NOT NULL REFERENCES plan_steps (id) ON DELETE CASCADE,
    posting_id BIGINT NOT NULL REFERENCES job_postings (id) ON DELETE CASCADE,
    rank INT NOT NULL CHECK (rank >= 1),
    score INT NOT NULL,
    band TEXT NOT NULL,
    PRIMARY KEY (step_id, posting_id)
);
