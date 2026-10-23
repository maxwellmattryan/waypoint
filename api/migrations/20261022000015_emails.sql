-- Recruiter email (api/src/email/, docs/email.md): one row per ingested message, grouped
-- into threads. `raw` keeps the original RFC 822 bytes so a later normalizer or heuristics
-- version can derive the other columns again; `normalizer_version` and
-- `heuristics_version` say which version wrote them.
--
-- `(source, provider_message_id)` identifies a message, so ingesting it again updates the
-- derived columns in place. A thread joins messages by Gmail thread id, then by
-- `Message-ID`/`In-Reply-To`/`References`, then by a reply's normalized `subject_key`.
-- `verdict` says whether a message is worth a model call; `verdict_reasons` lists every
-- heuristic rule code that fired. Every UPDATE sets `updated_at = now()` itself.

CREATE TABLE email_threads (
    id BIGSERIAL PRIMARY KEY,
    provider_thread_id TEXT UNIQUE,          -- Gmail threadId when known
    subject TEXT NOT NULL,                   -- display subject of the first message, prefixes removed
    subject_key TEXT NOT NULL,               -- lowercased, for the reply-subject fallback
    last_message_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX email_threads_subject_key_idx ON email_threads (subject_key, last_message_at DESC);

CREATE TABLE emails (
    id BIGSERIAL PRIMARY KEY,
    source TEXT NOT NULL CHECK (source IN ('fixture','gmail')),
    provider_message_id TEXT NOT NULL,
    thread_id BIGINT NOT NULL REFERENCES email_threads(id) ON DELETE CASCADE,
    rfc_message_id TEXT,
    in_reply_to TEXT[] NOT NULL DEFAULT '{}',
    references_ids TEXT[] NOT NULL DEFAULT '{}',
    labels TEXT[] NOT NULL DEFAULT '{}',     -- Gmail labelIds (#65)
    from_address TEXT,
    from_name TEXT,
    to_addresses TEXT[] NOT NULL DEFAULT '{}',
    cc_addresses TEXT[] NOT NULL DEFAULT '{}',
    subject TEXT NOT NULL DEFAULT '',
    sent_at TIMESTAMPTZ,
    headers JSONB NOT NULL,                  -- [{"name","value"}] in order, decoded
    raw BYTEA NOT NULL,
    body_text TEXT NOT NULL,
    body_source TEXT NOT NULL CHECK (body_source IN ('plain','html','none')),
    signature TEXT,
    verdict TEXT NOT NULL CHECK (verdict IN ('candidate','skip')),
    verdict_reasons JSONB NOT NULL CHECK (jsonb_typeof(verdict_reasons) = 'array'),
    ats TEXT CHECK (ats IN ('greenhouse','lever','ashby','workday')),
    normalizer_version INT NOT NULL,
    heuristics_version INT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source, provider_message_id)
);
CREATE INDEX emails_rfc_message_id_idx ON emails (rfc_message_id);
CREATE INDEX emails_references_idx ON emails USING GIN (references_ids);
CREATE INDEX emails_thread_idx ON emails (thread_id, sent_at);
CREATE INDEX emails_verdict_idx ON emails (verdict, id DESC);
