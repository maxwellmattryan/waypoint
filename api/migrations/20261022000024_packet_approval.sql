-- Packet approval and mark applied (api/src/workflows/packet_actions.rs,
-- docs/application-prep.md). A `ready` packet is approved through the approval gate of an
-- `approve_packet` run, which may edit the resume choice and the answers, and becomes
-- `approved`; marking it applied (an `apply_packet` run) records the user's own submission
-- and makes it `applied`. The draft columns (`resume_version_id`, `answers`) never change:
-- the `approved_*` columns hold what the user approved, `approved_resume_name` keeping the
-- resume's name once that resume is deleted. `application_id` links the application that
-- mark applied created or updated, so the application reaches the answers used through
-- this column; `applied_at` is when the packet was marked applied. Every UPDATE sets
-- `updated_at = now()` itself.

ALTER TABLE application_packets DROP CONSTRAINT application_packets_status_check;
ALTER TABLE application_packets ADD CONSTRAINT application_packets_status_check
    CHECK (status IN ('ready','in_review','dismissed','approved','applied'));
ALTER TABLE application_packets
    ADD COLUMN approved_resume_version_id BIGINT REFERENCES resume_versions (id) ON DELETE SET NULL,
    ADD COLUMN approved_resume_name TEXT,
    ADD COLUMN approved_answers JSONB,
    ADD COLUMN approved_at TIMESTAMPTZ,
    ADD COLUMN application_id BIGINT REFERENCES applications (id) ON DELETE SET NULL,
    ADD COLUMN applied_at TIMESTAMPTZ,
    ADD CONSTRAINT application_packets_approved_check CHECK (status NOT IN ('approved','applied')
        OR (approved_at IS NOT NULL AND approved_resume_name IS NOT NULL AND approved_answers IS NOT NULL)),
    ADD CONSTRAINT application_packets_applied_check CHECK ((status = 'applied') = (applied_at IS NOT NULL));

CREATE INDEX application_packets_application_idx ON application_packets (application_id)
    WHERE application_id IS NOT NULL;
