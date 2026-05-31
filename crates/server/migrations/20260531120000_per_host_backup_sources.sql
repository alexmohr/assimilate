-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Add per-host backup sources: allow backup_sources to be scoped to a specific
-- client within a schedule. NULL client_id means schedule-level (applies to all targets).

ALTER TABLE backup_sources ADD COLUMN client_id BIGINT REFERENCES clients(id) ON DELETE CASCADE;

-- Drop the old unique constraint (schedule_id, path) and replace with one that
-- includes client_id so the same path can exist for different hosts.
ALTER TABLE backup_sources DROP CONSTRAINT backup_sources_schedule_id_path_key;
CREATE UNIQUE INDEX backup_sources_schedule_client_path_idx
    ON backup_sources (schedule_id, COALESCE(client_id, -1), path);
