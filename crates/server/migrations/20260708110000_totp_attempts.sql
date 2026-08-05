-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Account-level (not just per-IP) brute-force tracking for TOTP code and
-- recovery-code verification, mirroring login_attempts for the password
-- step. Keyed by user_id rather than username since these attempts only
-- happen after a temp_token already identifies a specific user.
CREATE TABLE totp_attempts (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    ip TEXT NOT NULL,
    attempted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    success BOOLEAN NOT NULL DEFAULT false
);

CREATE INDEX idx_totp_attempts_user_id_attempted_at ON totp_attempts (user_id, attempted_at);
