-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Track when a repository was last synced (full borg scan)
ALTER TABLE repos ADD COLUMN last_synced_at TIMESTAMPTZ;

-- Remove duplicate archive imports before enforcing uniqueness.
-- Keep the row with the highest id (most recent insert) for each (repo, client, start) triple.
DELETE FROM backup_reports br
 WHERE br.id NOT IN (
     SELECT MAX(id)
       FROM backup_reports
      GROUP BY repo_id, client_id, started_at
 );

-- Prevent duplicate archive imports: same repo + client + start time = same archive
CREATE UNIQUE INDEX idx_backup_reports_dedup
    ON backup_reports (repo_id, client_id, started_at);
