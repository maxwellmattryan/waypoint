-- One row per uploaded resume PDF. The bytes live here (BYTEA) rather than on disk, so a
-- delete is transactional with the row and there are no file paths. `text` is the
-- extracted text, or NULL when extraction failed. Rows are never updated.
-- Name and filename length limits and the 10 MiB size limit are enforced in the API
-- (api/src/resumes.rs), not here.
CREATE TABLE resume_versions (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    filename TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes > 0),
    sha256 TEXT NOT NULL,
    pdf BYTEA NOT NULL,
    text TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX resume_versions_created_at_idx ON resume_versions (created_at DESC, id DESC);

-- The resume version sent with an application. RESTRICT keeps a linked version from being
-- deleted, so the "sent with" history can't silently disappear.
ALTER TABLE applications
    ADD COLUMN resume_version_id BIGINT REFERENCES resume_versions (id) ON DELETE RESTRICT;

CREATE INDEX applications_resume_version_id_idx ON applications (resume_version_id);
