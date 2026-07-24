-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Add pending_totp column to sessions for TOTP pre-verification temp sessions
ALTER TABLE sessions ADD COLUMN pending_totp BOOLEAN NOT NULL DEFAULT false;

-- Update existing temp sessions (those with TOTP-required users that are expiring
-- within 10 minutes from created_at) to have pending_totp = true.
-- This handles any temp sessions created before this migration.
-- Temp sessions have expires_at = created_at + 10 minutes and remember_me = false
UPDATE sessions SET pending_totp = true
WHERE remember_me = false
  AND expires_at <= created_at + INTERVAL '10 minutes'
  AND expires_at > NOW();
