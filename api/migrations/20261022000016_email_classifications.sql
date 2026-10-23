-- Classify and extract (api/src/workflows/classify_email.rs, docs/email.md): one row per
-- candidate email and input key. `input_key` is a SHA-256 over the email fields the
-- prompt reads plus `classifier_version`, so re-ingesting an unchanged message reuses its
-- row and a changed one gets a new row (the older pending row becomes `superseded`).
--
-- A row starts `pending` with the `classify_email` run that will fill it. A valid answer
-- makes it `classified` with the normalized fields; an answer that breaks the validation
-- rules makes it `invalid` with fixed-string `invalid_reasons`. `issues` lists the fields
-- validation dropped or kept unconverted. Every UPDATE sets `updated_at = now()` itself.

CREATE TABLE email_classifications (
    id BIGSERIAL PRIMARY KEY,
    email_id BIGINT NOT NULL REFERENCES emails (id) ON DELETE CASCADE,
    run_id BIGINT REFERENCES agent_runs (id) ON DELETE SET NULL,
    input_key TEXT NOT NULL,
    classifier_version INT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending','classified','invalid','superseded')),
    class TEXT CHECK (class IN ('rejection','interview_invite','availability_request','offer','other')),
    confidence DOUBLE PRECISION CHECK (confidence >= 0 AND confidence <= 1),
    company TEXT,
    role TEXT,
    stage TEXT,
    interviewer TEXT,
    proposed_times JSONB NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(proposed_times) = 'array'),
    issues JSONB NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(issues) = 'array'),
    invalid_reasons JSONB NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(invalid_reasons) = 'array'),
    claude_call_id BIGINT REFERENCES claude_calls (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (email_id, input_key),
    CHECK (status <> 'classified' OR (class IS NOT NULL AND confidence IS NOT NULL))
);
CREATE INDEX email_classifications_email_idx ON email_classifications (email_id, id DESC);
