-- One row per tracked job application. Every UPDATE sets `updated_at = now()` itself
-- (no trigger); `status_changed_at` moves only when the status actually changes.
-- Text length limits are enforced in the API (api/src/applications.rs), not here.
CREATE TABLE applications (
    id BIGSERIAL PRIMARY KEY,
    company TEXT NOT NULL,
    title TEXT NOT NULL,
    url TEXT,
    location TEXT,
    comp TEXT,
    source TEXT,
    notes TEXT,
    status TEXT NOT NULL DEFAULT 'sourced'
        CHECK (status IN ('sourced', 'applied', 'interviewing', 'offer', 'rejected')),
    status_changed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX applications_updated_at_idx ON applications (updated_at DESC, id DESC);
