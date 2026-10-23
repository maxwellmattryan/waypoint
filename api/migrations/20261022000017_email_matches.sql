-- Match and act (api/src/workflows/match_email.rs, docs/email.md): one row per classified
-- email, queued with the `match_email` run that fills it. `status` starts `pending` and
-- ends as the run decided: `applied` (its status and stage changes were made, or the user
-- accepted its review item), `no_action` (linked, nothing to change), `review` (a review
-- item waits for the user), `unmatched` or `skipped`. `application_id` is the linked
-- application; `applied` and `no_action` rows feed the `thread` and `sender_domain`
-- signals of later emails. `candidates`, `reasons` and `ops` keep what the matcher saw and
-- planned. Every UPDATE sets `updated_at = now()` itself.
--
-- `application_events.source_email_id` names the email that caused an event, so the
-- timeline can show it; it becomes NULL when the email is deleted.

CREATE TABLE email_matches (
    id BIGSERIAL PRIMARY KEY,
    classification_id BIGINT NOT NULL UNIQUE REFERENCES email_classifications (id) ON DELETE CASCADE,
    email_id BIGINT NOT NULL REFERENCES emails (id) ON DELETE CASCADE,
    run_id BIGINT REFERENCES agent_runs (id) ON DELETE SET NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending','applied','no_action','review','unmatched','skipped')),
    application_id BIGINT REFERENCES applications (id) ON DELETE SET NULL,
    confidence DOUBLE PRECISION CHECK (confidence >= 0 AND confidence <= 1),
    reasons JSONB NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(reasons) = 'array'),
    candidates JSONB NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(candidates) = 'array'),
    ops JSONB NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(ops) = 'array'),
    review_item_id BIGINT REFERENCES review_items (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX email_matches_application_idx ON email_matches (application_id) WHERE application_id IS NOT NULL;
CREATE INDEX email_matches_email_idx ON email_matches (email_id);

ALTER TABLE application_events ADD COLUMN source_email_id BIGINT REFERENCES emails (id) ON DELETE SET NULL;
CREATE INDEX application_events_source_email_idx ON application_events (source_email_id) WHERE source_email_id IS NOT NULL;
