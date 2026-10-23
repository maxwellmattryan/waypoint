-- Board sourcing (api/src/sourcing, docs/sourcing.md): one row per posting seen on a
-- target company's public Greenhouse, Lever or Ashby board, kept in step with the board by
-- the `source_board` job. `(source, external_id)` is unique, so re-sourcing a board
-- updates its rows in place. `first_seen_at` is when the posting first appeared;
-- `last_seen_at` is the last successful fetch that listed it. A posting the board stopped
-- listing gets `removed_at`, and one that reappears gets `removed_at = NULL` again. The
-- normalized location (`remote`, `city`, `region`, `country`) sits beside the board's raw
-- text. Every UPDATE sets `updated_at = now()` itself.

CREATE TABLE job_postings (
    id              bigserial PRIMARY KEY,
    source          text NOT NULL CHECK (source IN ('greenhouse', 'lever', 'ashby')),
    board           text NOT NULL,
    external_id     text NOT NULL,
    title           text NOT NULL,
    company         text NOT NULL,
    url             text NOT NULL,
    apply_url       text,
    department      text,
    employment_type text,
    location_raw    text,
    remote          boolean NOT NULL DEFAULT false,
    city            text,
    region          text,
    country         text,
    description     text NOT NULL DEFAULT '',
    posted_at       timestamptz,
    first_seen_at   timestamptz NOT NULL DEFAULT now(),
    last_seen_at    timestamptz NOT NULL DEFAULT now(),
    removed_at      timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (source, external_id)
);

CREATE INDEX job_postings_live_board ON job_postings (source, board) WHERE removed_at IS NULL;
