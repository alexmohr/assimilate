-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Add TOTP/2FA columns to users table
ALTER TABLE users ADD COLUMN totp_secret_encrypted BYTEA;
ALTER TABLE users ADD COLUMN totp_enabled BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE users ADD COLUMN totp_recovery_codes TEXT[];
-- Time-step (unix_time / 30s) of the last TOTP code successfully consumed
-- during login, used for replay protection.
ALTER TABLE users ADD COLUMN totp_last_verified_step BIGINT;

-- Add last_seen_at to sessions table for idle timeout tracking. Added
-- nullable first so existing sessions can be backfilled to their
-- created_at before the NOT NULL constraint is applied - adding it
-- directly with `NOT NULL DEFAULT NOW()` would evaluate NOW() once for
-- the whole statement, backfilling every existing row to the migration's
-- apply time instead of each session's own created_at.
ALTER TABLE sessions ADD COLUMN last_seen_at TIMESTAMPTZ;
UPDATE sessions SET last_seen_at = created_at WHERE last_seen_at IS NULL;
ALTER TABLE sessions ALTER COLUMN last_seen_at SET NOT NULL;
ALTER TABLE sessions ALTER COLUMN last_seen_at SET DEFAULT NOW();

-- Seed the session idle timeout setting (default: 480 minutes = 8 hours)
INSERT INTO system_settings (key, value) VALUES ('session_idle_timeout_minutes', '480')
ON CONFLICT (key) DO NOTHING;
