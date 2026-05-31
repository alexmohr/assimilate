-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Per-host exclude patterns: allow exclude patterns to be scoped to a specific
-- client within a schedule. NULL client_id means schedule-level (applies to all targets).

CREATE TABLE schedule_excludes (
    id BIGSERIAL PRIMARY KEY,
    schedule_id BIGINT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    client_id BIGINT REFERENCES clients(id) ON DELETE CASCADE,
    pattern TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE UNIQUE INDEX schedule_excludes_schedule_client_pattern_idx
    ON schedule_excludes (schedule_id, COALESCE(client_id, -1), pattern);
