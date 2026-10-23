-- What Waypoint knows about the user: facts, preferences and answer-bank entries, each
-- with where it came from and whether the user has confirmed it (api/src/memories.rs).
-- Provenance is deliberately generic: `source_kind` names the source and `source_ref` is
-- an id in it as text (a resume version id, a run id, a Gmail message id), with no
-- foreign key per source table, so later sources need no migration and deleting a resume
-- or run keeps what was learned from it. Content and tag lengths are enforced in
-- api/src/memories.rs; the CHECKs here keep kind/topic pairs, statuses and sources valid
-- whoever writes the row.

-- `array_to_string` is only STABLE, which a generated column refuses; this wrapper is
-- IMMUTABLE because its result depends on its arguments alone.
CREATE FUNCTION memory_search_vector(content TEXT, topic TEXT, tags TEXT[])
RETURNS tsvector
LANGUAGE sql
IMMUTABLE
PARALLEL SAFE
RETURN setweight(to_tsvector('english'::regconfig, content), 'A')
    || setweight(
        to_tsvector('english'::regconfig, replace(topic, '_', ' ') || ' ' || array_to_string(tags, ' ')),
        'B'
    );

CREATE TABLE memories (
    id BIGSERIAL PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('fact','preference','answer')),
    topic TEXT NOT NULL,
    content TEXT NOT NULL,
    tags TEXT[] NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'confirmed' CHECK (status IN ('proposed','confirmed')),
    confirmed_at TIMESTAMPTZ,
    source_kind TEXT NOT NULL DEFAULT 'manual' CHECK (source_kind IN ('manual','resume','linkedin','run','email')),
    source_ref TEXT,
    source_label TEXT,
    search tsvector GENERATED ALWAYS AS (memory_search_vector(content, topic, tags)) STORED,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT memories_topic_check CHECK (
        (kind = 'fact' AND topic IN ('position','skill','education','other'))
        OR (kind = 'preference' AND topic IN ('target_role','seniority','location','comp_floor','industry','deal_breaker','other'))
        OR (kind = 'answer' AND topic IN ('work_authorization','notice_period','salary_expectation','why_leaving','other'))
    )
);

CREATE INDEX memories_search_idx ON memories USING GIN (search);
CREATE INDEX memories_tags_idx ON memories USING GIN (tags);
CREATE INDEX memories_updated_at_idx ON memories (updated_at DESC, id DESC);
