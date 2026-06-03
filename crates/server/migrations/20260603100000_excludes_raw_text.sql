-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Migrate global excludes to a single raw text value, preserving order.
CREATE TABLE excludes_global_config (
    id   BOOLEAN NOT NULL DEFAULT TRUE PRIMARY KEY,
    raw_text TEXT NOT NULL DEFAULT '',
    CONSTRAINT excludes_global_config_singleton CHECK (id = TRUE)
);
INSERT INTO excludes_global_config (raw_text)
    SELECT COALESCE(string_agg(pattern, E'\n' ORDER BY sort_order, id), '')
    FROM excludes_global;
DROP TABLE excludes_global;

-- Replace schedule-level exclude_patterns TEXT[] with raw text storage.
ALTER TABLE schedules ADD COLUMN exclude_patterns_raw TEXT NOT NULL DEFAULT '';
UPDATE schedules SET exclude_patterns_raw = array_to_string(exclude_patterns, E'\n');
ALTER TABLE schedules DROP COLUMN exclude_patterns;

-- Replace per-host exclude rows with one raw-text row per (schedule, client).
CREATE TABLE per_host_excludes (
    id BIGSERIAL PRIMARY KEY,
    schedule_id BIGINT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    client_id  BIGINT NOT NULL REFERENCES clients(id)  ON DELETE CASCADE,
    raw_text   TEXT NOT NULL DEFAULT '',
    UNIQUE (schedule_id, client_id)
);

INSERT INTO per_host_excludes (schedule_id, client_id, raw_text)
    SELECT schedule_id, client_id,
           string_agg(pattern, E'\n' ORDER BY sort_order, id)
    FROM schedule_excludes
    WHERE client_id IS NOT NULL
    GROUP BY schedule_id, client_id;

DROP TABLE schedule_excludes;
