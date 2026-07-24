-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE users ADD COLUMN locked_until TIMESTAMPTZ;

CREATE INDEX idx_login_attempts_username_attempted
    ON login_attempts(username, attempted_at DESC);
