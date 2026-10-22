-- One row per `claude -p` call, successful or not. `duration_ms` is the runner's
-- wall-clock time; `cost_usd` is the CLI's reported `total_cost_usd`.
CREATE TABLE claude_calls (
    id BIGSERIAL PRIMARY KEY,
    purpose TEXT NOT NULL,
    model TEXT,
    status TEXT NOT NULL CHECK (status IN ('ok','usage_limit','cli_error','schema_mismatch','timeout','process_failed','protocol_error')),
    input_tokens BIGINT,
    output_tokens BIGINT,
    cache_creation_input_tokens BIGINT,
    cache_read_input_tokens BIGINT,
    duration_ms BIGINT NOT NULL,
    cost_usd DOUBLE PRECISION,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX claude_calls_created_at_idx ON claude_calls (created_at);
