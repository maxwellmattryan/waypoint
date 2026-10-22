-- Per-application interview stages and an append-only event log (the timeline). Both go
-- with their application (ON DELETE CASCADE). Text length limits are enforced in the API
-- (api/src/applications/), not here. `stage_name` is a snapshot of the stage's name, so a
-- removed stage still reads in the timeline. The unique position is deferred so a reorder
-- can swap positions inside one statement.
CREATE TABLE application_stages (
    id BIGSERIAL PRIMARY KEY,
    application_id BIGINT NOT NULL REFERENCES applications (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (application_id, position) DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE application_events (
    id BIGSERIAL PRIMARY KEY,
    application_id BIGINT NOT NULL REFERENCES applications (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('created', 'status_changed', 'stage_added',
        'stage_completed', 'stage_reopened', 'stage_removed', 'note', 'rejection_feedback')),
    from_status TEXT
        CHECK (from_status IN ('sourced', 'applied', 'interviewing', 'offer', 'rejected')),
    to_status TEXT
        CHECK (to_status IN ('sourced', 'applied', 'interviewing', 'offer', 'rejected')),
    stage_name TEXT,
    body TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX application_events_timeline_idx
    ON application_events (application_id, created_at DESC, id DESC);

-- Applications that existed before the log get one `created` event at their creation time.
-- Their starting status is unknown, so `to_status` stays null.
INSERT INTO application_events (application_id, kind, created_at)
    SELECT id, 'created', created_at FROM applications;
