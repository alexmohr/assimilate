-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Add TOTP/2FA columns to users table
ALTER TABLE users ADD COLUMN totp_secret_encrypted BYTEA;
ALTER TABLE users ADD COLUMN totp_enabled BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE users ADD COLUMN totp_recovery_codes TEXT[];
ALTER TABLE users ADD COLUMN totp_last_verified_at TIMESTAMPTZ;

-- Add last_seen_at to sessions table for idle timeout tracking
ALTER TABLE sessions ADD COLUMN last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- Update existing sessions to have last_seen_at set to their created_at
UPDATE sessions SET last_seen_at = created_at WHERE last_seen_at IS NULL;

-- Seed the session idle timeout setting (default: 480 minutes = 8 hours)
INSERT INTO system_settings (key, value) VALUES ('session_idle_timeout_minutes', '480')
ON CONFLICT (key) DO NOTHING;
