-- Gmail sync progress (one row at most). The page cursor lives in the gmail_sync job's payload.
CREATE TABLE gmail_sync_state (
    id BOOLEAN PRIMARY KEY DEFAULT true CHECK (id),
    email_address TEXT,                -- the account the history_id belongs to
    history_id TEXT,                   -- set only when a run's last page finishes
    run_mode TEXT CHECK (run_mode IN ('backfill','resync','incremental')),
    run_started_at TIMESTAMPTZ,
    run_seen INT NOT NULL DEFAULT 0, run_inserted INT NOT NULL DEFAULT 0,
    run_updated INT NOT NULL DEFAULT 0, run_oversized INT NOT NULL DEFAULT 0,
    run_failed INT NOT NULL DEFAULT 0, run_vanished INT NOT NULL DEFAULT 0,
    last_finished_at TIMESTAMPTZ,
    last_error TEXT, last_error_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
