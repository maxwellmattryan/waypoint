-- Learning signals: one row per decision the user made on something an agent drafted, the
-- raw evidence a later memory writer learns from (api/src/memory_signals.rs). An approval
-- keeps the drafted input, the approved one and the diff between them; a rejection keeps
-- what was drafted; a dismissal keeps the review item's payload and the user's optional
-- reason. Each row is written in the same transaction as the decision that causes it.
-- There are deliberately no foreign keys, as with `memories`' provenance: a signal outlives
-- the run, pending action or review item it came from. The UNIQUE columns make "one signal
-- per decision" a database fact, and `memory_signals_shape_check` keeps each kind's columns
-- consistent whoever writes the row. (`changes IS NOT NULL` is spelled out because
-- `jsonb_typeof(NULL) = 'array'` is NULL, and a CHECK that is NULL passes.)

CREATE TABLE memory_signals (
    id BIGSERIAL PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('approval','rejection','dismissal')),
    run_id BIGINT NOT NULL,
    pending_action_id BIGINT UNIQUE,
    review_item_id BIGINT UNIQUE,
    subject TEXT NOT NULL,          -- the tool name, or the review item's kind
    drafted JSONB NOT NULL,         -- the action's input, or the item's payload
    approved JSONB,                 -- approvals only: approved_input
    changes JSONB,                  -- approvals only: the diff array
    reason TEXT CHECK (char_length(reason) BETWEEN 1 AND 1000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT memory_signals_shape_check CHECK (
        (kind = 'approval'  AND pending_action_id IS NOT NULL AND review_item_id IS NULL
            AND approved IS NOT NULL AND changes IS NOT NULL
            AND jsonb_typeof(changes) = 'array' AND reason IS NULL)
     OR (kind = 'rejection' AND pending_action_id IS NOT NULL AND review_item_id IS NULL
            AND approved IS NULL AND changes IS NULL AND reason IS NULL)
     OR (kind = 'dismissal' AND review_item_id IS NOT NULL AND pending_action_id IS NULL
            AND approved IS NULL AND changes IS NULL)
    )
);

CREATE INDEX memory_signals_kind_idx ON memory_signals (kind, id DESC);
