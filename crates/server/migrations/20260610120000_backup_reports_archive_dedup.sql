-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Borg's `list --json` reports each archive `start` with whole-second precision,
-- so two distinct archives of the same host created within the same second
-- shared a (repo_id, client_id, started_at) key. The import/resync bulk insert
-- used that triple as its `ON CONFLICT` arbiter with `DO NOTHING`, so the second
-- archive was silently dropped: it showed up in the import progress log but
-- vanished from the database, and a full resync would not bring it back.
--
-- The fix is to include the archive name in the dedup key for rows that carry
-- one. Distinct archive names no longer collide even when they share a client
-- and start second, while re-importing the same archive is still idempotent.
-- Reports without an archive name (pending/started/failed agent reports) keep
-- the original triple via a partial index.

DROP INDEX IF EXISTS idx_backup_reports_dedup;

-- Pending/started/failed reports (no archive name) are deduplicated per run.
CREATE UNIQUE INDEX idx_backup_reports_dedup
    ON backup_reports (repo_id, client_id, started_at)
    WHERE archive_name IS NULL;

-- Imported/synced archives are deduplicated by name as well, so distinct
-- archives sharing a start second remain distinct rows.
CREATE UNIQUE INDEX idx_backup_reports_archive_dedup
    ON backup_reports (repo_id, client_id, started_at, archive_name)
    WHERE archive_name IS NOT NULL;
