-- Calendar interview proposals (api/src/workflows/propose_interview.rs, docs/email.md):
-- one row per future interview time ("slot") of a classification once it is linked to an
-- application. Each row is queued with the `propose_interview` run that asks the user to
-- approve a `create_calendar_event` action. `request` is the tool input built once, at
-- queue time, so a rerun of the step finds the same pending action. `status` starts
-- `proposed` and ends `created` (the event is in Google Calendar: `google_event_id`,
-- `html_link`, `start_at`/`end_at` as created), `rejected` (the user rejected it) or
-- `failed` (the Calendar call failed; `error` says why, it is never retried).
--
-- `(application_id, proposed_start)` is unique, so a time is proposed once per
-- application: a later email repeating it, or a re-ingested one, adds nothing, and a
-- rejected or failed time is not proposed again. Rows go with their application.
-- Every UPDATE sets `updated_at = now()` itself.
--
-- `application_events.kind` gains `interview_scheduled`: the event was created, and its
-- `body` names the summary and the time.

CREATE TABLE interview_events (
    id BIGSERIAL PRIMARY KEY,
    application_id BIGINT NOT NULL REFERENCES applications (id) ON DELETE CASCADE,
    email_id BIGINT REFERENCES emails (id) ON DELETE SET NULL,
    classification_id BIGINT REFERENCES email_classifications (id) ON DELETE SET NULL,
    run_id BIGINT REFERENCES agent_runs (id) ON DELETE SET NULL,
    status TEXT NOT NULL DEFAULT 'proposed'
        CHECK (status IN ('proposed','created','rejected','failed')),
    proposed_start TIMESTAMPTZ NOT NULL,
    request JSONB NOT NULL CHECK (jsonb_typeof(request) = 'object'),
    start_at TIMESTAMPTZ,
    end_at TIMESTAMPTZ,
    google_event_id TEXT,
    html_link TEXT,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (application_id, proposed_start),
    CHECK (status <> 'created' OR google_event_id IS NOT NULL)
);
CREATE INDEX interview_events_run_idx ON interview_events (run_id) WHERE run_id IS NOT NULL;

ALTER TABLE application_events DROP CONSTRAINT application_events_kind_check;
ALTER TABLE application_events ADD CONSTRAINT application_events_kind_check CHECK (kind IN
    ('created','status_changed','stage_added','stage_completed','stage_reopened','stage_removed',
     'note','rejection_feedback','interview_scheduled'));
