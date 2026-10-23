-- Fit scoring (api/src/workflows/score_fit.rs, docs/sourcing.md): one row per `score_fit`
-- run that produced a valid score for a posting. `status` is `applied` (the score was
-- confident and every citation checked out, or the user accepted its review item),
-- `in_review` (a `score_fit.score` review item waits for the user) or `dismissed` (the user
-- dismissed that item). A posting's current score is its latest `applied` row. `band` is
-- computed in Rust from `score`. `reasons` holds the validated reasons with a snapshot of
-- each cited memory. `cost_usd` is the model call's reported cost. Every UPDATE sets
-- `updated_at = now()` itself.

CREATE TABLE posting_fit_scores (
    id BIGSERIAL PRIMARY KEY,
    posting_id BIGINT NOT NULL REFERENCES job_postings (id) ON DELETE CASCADE,
    run_id BIGINT NOT NULL UNIQUE REFERENCES agent_runs (id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('applied','in_review','dismissed')),
    score INT NOT NULL CHECK (score BETWEEN 0 AND 100),
    band TEXT NOT NULL CHECK (band IN ('strong','partial','weak')),
    confidence DOUBLE PRECISION NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    summary TEXT NOT NULL,
    reasons JSONB NOT NULL,
    review_item_id BIGINT UNIQUE REFERENCES review_items (id) ON DELETE SET NULL,
    claude_call_id BIGINT REFERENCES claude_calls (id) ON DELETE SET NULL,
    cost_usd DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (cost_usd >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX posting_fit_scores_posting_idx ON posting_fit_scores (posting_id, status, id DESC);
