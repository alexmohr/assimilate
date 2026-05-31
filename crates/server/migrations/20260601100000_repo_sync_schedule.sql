-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Add per-repo sync schedule (cron expression).
-- NULL means sync from disk is disabled; default is twice daily.
ALTER TABLE repos ADD COLUMN sync_schedule TEXT DEFAULT '0 0,12 * * *';
