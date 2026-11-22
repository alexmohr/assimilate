-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Single-use, short-lived tokens a new host trades for an agent token on its
-- first connection. Only the SHA-256 of the token is kept: it carries 32 random
-- bytes, so a fast hash is enough and lets the server look a presented token up.
-- A token is pending until it is used, revoked or past `expires_at`; the status
-- is derived from those columns rather than stored.
CREATE TABLE enrollment_tokens (
    id BIGSERIAL PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    hint TEXT NOT NULL,
    created_by BIGINT REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    hostname TEXT,
    tag_ids BIGINT[] NOT NULL DEFAULT '{}',
    used_at TIMESTAMPTZ,
    used_by_agent_id BIGINT REFERENCES agents(id) ON DELETE SET NULL,
    revoked_at TIMESTAMPTZ,
    CHECK (used_at IS NULL OR revoked_at IS NULL)
);

CREATE INDEX idx_enrollment_tokens_created_at ON enrollment_tokens (created_at DESC);
CREATE INDEX idx_enrollment_tokens_created_by ON enrollment_tokens (created_by);
CREATE INDEX idx_enrollment_tokens_used_by_agent_id ON enrollment_tokens (used_by_agent_id);
