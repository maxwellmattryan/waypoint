-- The connected Google account (one row at most). Tokens are XChaCha20-Poly1305 blobs
-- sealed with WAYPOINT_TOKEN_KEY (see api/src/crypto.rs); never plaintext.
CREATE TABLE google_accounts (
    id BOOLEAN PRIMARY KEY DEFAULT true CHECK (id),
    refresh_token BYTEA NOT NULL,
    access_token BYTEA NOT NULL,
    access_token_expires_at TIMESTAMPTZ NOT NULL,
    scopes TEXT[] NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('connected','reconnect_required')),
    connected_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One row per started connect flow; consumed by the callback, expired after 10 minutes.
CREATE TABLE google_oauth_states (
    state TEXT PRIMARY KEY,
    code_verifier TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
