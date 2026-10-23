-- Application packets (api/src/workflows/prep_packet.rs, docs/application-prep.md): one row
-- per `prep_packet` run that produced a valid packet for a posting, stored through the
-- `save_application_packet` write tool. `status` is `ready` (confident, every answer
-- grounded and nothing dropped, or the user accepted its review item), `in_review` (a
-- `prep_packet.packet` review item waits for the user) or `dismissed` (the user dismissed
-- that item). Job 81 extends the CHECK with its own statuses. `resume_reasons` and `answers`
-- hold the validated reasons and the five answers with a snapshot of each cited memory;
-- `resume_name` keeps the chosen resume's name once that resume is deleted. The `fit_*`
-- columns snapshot the posting's applied fit score when the packet was prepared.
-- `cost_usd` is the model call's reported cost. Every UPDATE sets `updated_at = now()`
-- itself.

CREATE TABLE application_packets (
    id BIGSERIAL PRIMARY KEY,
    posting_id BIGINT NOT NULL REFERENCES job_postings (id) ON DELETE CASCADE,
    run_id BIGINT NOT NULL UNIQUE REFERENCES agent_runs (id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('ready','in_review','dismissed')),
    resume_version_id BIGINT REFERENCES resume_versions (id) ON DELETE SET NULL,
    resume_name TEXT NOT NULL,
    resume_reasons JSONB NOT NULL,
    answers JSONB NOT NULL,
    fit_score_id BIGINT REFERENCES posting_fit_scores (id) ON DELETE SET NULL,
    fit_score INT CHECK (fit_score BETWEEN 0 AND 100),
    fit_band TEXT CHECK (fit_band IN ('strong','partial','weak')),
    confidence DOUBLE PRECISION NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    review_item_id BIGINT UNIQUE REFERENCES review_items (id) ON DELETE SET NULL,
    claude_call_id BIGINT REFERENCES claude_calls (id) ON DELETE SET NULL,
    cost_usd DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (cost_usd >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX application_packets_posting_idx ON application_packets (posting_id, id DESC);
