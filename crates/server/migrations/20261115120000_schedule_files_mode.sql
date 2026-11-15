-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Where a backup schedule's files come from. The mode mirrors
-- shared::types::ScheduleFilesMode:
--   'host' backs up each host's own file set (Sources > Files): its paths,
--          excludes, include patterns, file change patterns and commands;
--   'own'  backs up the schedule's own file set instead, the same fields,
--          replacing each host's set as a whole;
--   'off'  reads no files, for a schedule that only dumps databases or
--          snapshots virtual machines.
-- run_host_commands lets a 'host' schedule skip the hosts' pre- and
-- post-backup commands.
ALTER TABLE schedules
    ADD COLUMN files_mode TEXT NOT NULL DEFAULT 'host'
        CHECK (files_mode IN ('host', 'own', 'off')),
    ADD COLUMN run_host_commands BOOLEAN NOT NULL DEFAULT TRUE;

-- A host's own include patterns: exceptions to its excludes, as a schedule
-- has had all along.
ALTER TABLE agents
    ADD COLUMN default_include_patterns TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[];

-- Until now a schedule's own paths replaced each host's, but the host's
-- excludes, file change patterns and commands were always combined with the
-- schedule's. An 'own' schedule now replaces the host's set as a whole, so
-- every schedule that set any file field of its own becomes 'own', and the
-- host values it used to inherit are written into its own set. Every backup
-- reads exactly what it read before.
CREATE TEMPORARY TABLE files_mode_own ON COMMIT DROP AS
SELECT s.id
FROM schedules s
WHERE s.schedule_type = 'backup'
  AND (
      EXISTS (SELECT 1 FROM backup_sources b WHERE b.schedule_id = s.id)
      OR btrim(s.exclude_patterns_raw) <> ''
      OR btrim(s.include_patterns_raw) <> ''
      OR btrim(s.file_change_patterns_raw) <> ''
      OR s.pre_backup_commands <> '[]'::JSONB
      OR s.post_backup_commands <> '[]'::JSONB
      OR EXISTS (SELECT 1 FROM per_agent_excludes p WHERE p.schedule_id = s.id)
      OR EXISTS (SELECT 1 FROM per_agent_includes p WHERE p.schedule_id = s.id)
      OR EXISTS (SELECT 1 FROM per_agent_file_change_patterns p WHERE p.schedule_id = s.id)
      OR EXISTS (SELECT 1 FROM per_agent_commands p WHERE p.schedule_id = s.id)
  );

UPDATE schedules SET files_mode = 'own' WHERE id IN (SELECT id FROM files_mode_own);

-- The file set each target host of an 'own' schedule actually ran with,
-- worked out the way the config assembler did before this migration.
CREATE TEMPORARY TABLE files_mode_sets ON COMMIT DROP AS
SELECT
    t.schedule_id,
    t.agent_id,
    COALESCE(
        (SELECT array_agg(b.path ORDER BY b.sort_order, b.id) FROM backup_sources b
         WHERE b.schedule_id = t.schedule_id AND b.agent_id = t.agent_id),
        (SELECT array_agg(b.path ORDER BY b.sort_order, b.id) FROM backup_sources b
         WHERE b.schedule_id = t.schedule_id AND b.agent_id IS NULL),
        a.default_backup_paths
    ) AS paths,
    concat_ws(
        E'\n',
        NULLIF(array_to_string(a.default_exclude_patterns, E'\n'), ''),
        NULLIF(COALESCE(pe.raw_text, s.exclude_patterns_raw), '')
    ) AS excludes,
    COALESCE(pi.raw_text, s.include_patterns_raw) AS includes,
    concat_ws(
        E'\n',
        NULLIF(COALESCE(pf.raw_text, s.file_change_patterns_raw), ''),
        NULLIF(a.default_file_change_patterns_raw, '')
    ) AS file_change_patterns,
    a.default_pre_backup_commands || COALESCE(pc.pre_backup_commands, s.pre_backup_commands)
        AS pre_backup_commands,
    COALESCE(pc.post_backup_commands, s.post_backup_commands) || a.default_post_backup_commands
        AS post_backup_commands
FROM schedule_targets t
JOIN files_mode_own o ON o.id = t.schedule_id
JOIN schedules s ON s.id = t.schedule_id
JOIN agents a ON a.id = t.agent_id
LEFT JOIN per_agent_excludes pe ON pe.schedule_id = t.schedule_id AND pe.agent_id = t.agent_id
LEFT JOIN per_agent_includes pi ON pi.schedule_id = t.schedule_id AND pi.agent_id = t.agent_id
LEFT JOIN per_agent_file_change_patterns pf
    ON pf.schedule_id = t.schedule_id AND pf.agent_id = t.agent_id
LEFT JOIN per_agent_commands pc ON pc.schedule_id = t.schedule_id AND pc.agent_id = t.agent_id;

-- A field every host of a schedule shares goes into the schedule itself; a
-- field the hosts differ in is kept per host.
CREATE TEMPORARY TABLE files_mode_shared ON COMMIT DROP AS
SELECT
    schedule_id,
    count(DISTINCT paths) = 1 AS paths,
    count(DISTINCT excludes) = 1 AS excludes,
    count(DISTINCT includes) = 1 AS includes,
    count(DISTINCT file_change_patterns) = 1 AS file_change_patterns,
    count(DISTINCT (pre_backup_commands, post_backup_commands)) = 1 AS commands
FROM files_mode_sets
GROUP BY schedule_id;

-- Paths.
DELETE FROM backup_sources WHERE schedule_id IN (SELECT schedule_id FROM files_mode_shared);

INSERT INTO backup_sources (schedule_id, agent_id, path, sort_order)
SELECT DISTINCT ON (m.schedule_id, p.path)
    m.schedule_id, NULL::BIGINT, p.path, (p.ord - 1)::INTEGER
FROM files_mode_sets m
JOIN files_mode_shared sh ON sh.schedule_id = m.schedule_id AND sh.paths
CROSS JOIN LATERAL unnest(m.paths) WITH ORDINALITY AS p(path, ord)
ORDER BY m.schedule_id, p.path, p.ord;

INSERT INTO backup_sources (schedule_id, agent_id, path, sort_order)
SELECT DISTINCT ON (m.schedule_id, m.agent_id, p.path)
    m.schedule_id, m.agent_id, p.path, (p.ord - 1)::INTEGER
FROM files_mode_sets m
JOIN files_mode_shared sh ON sh.schedule_id = m.schedule_id AND NOT sh.paths
CROSS JOIN LATERAL unnest(m.paths) WITH ORDINALITY AS p(path, ord)
ORDER BY m.schedule_id, m.agent_id, p.path, p.ord;

-- Excludes.
UPDATE schedules s
SET exclude_patterns_raw = m.excludes
FROM files_mode_sets m
JOIN files_mode_shared sh ON sh.schedule_id = m.schedule_id AND sh.excludes
WHERE s.id = m.schedule_id;

DELETE FROM per_agent_excludes
WHERE schedule_id IN (SELECT schedule_id FROM files_mode_shared);

INSERT INTO per_agent_excludes (schedule_id, agent_id, raw_text)
SELECT m.schedule_id, m.agent_id, m.excludes
FROM files_mode_sets m
JOIN files_mode_shared sh ON sh.schedule_id = m.schedule_id AND NOT sh.excludes;

-- Include patterns.
UPDATE schedules s
SET include_patterns_raw = m.includes
FROM files_mode_sets m
JOIN files_mode_shared sh ON sh.schedule_id = m.schedule_id AND sh.includes
WHERE s.id = m.schedule_id;

DELETE FROM per_agent_includes
WHERE schedule_id IN (SELECT schedule_id FROM files_mode_shared);

INSERT INTO per_agent_includes (schedule_id, agent_id, raw_text)
SELECT m.schedule_id, m.agent_id, m.includes
FROM files_mode_sets m
JOIN files_mode_shared sh ON sh.schedule_id = m.schedule_id AND NOT sh.includes;

-- File change patterns.
UPDATE schedules s
SET file_change_patterns_raw = m.file_change_patterns
FROM files_mode_sets m
JOIN files_mode_shared sh ON sh.schedule_id = m.schedule_id AND sh.file_change_patterns
WHERE s.id = m.schedule_id;

DELETE FROM per_agent_file_change_patterns
WHERE schedule_id IN (SELECT schedule_id FROM files_mode_shared);

INSERT INTO per_agent_file_change_patterns (schedule_id, agent_id, raw_text)
SELECT m.schedule_id, m.agent_id, m.file_change_patterns
FROM files_mode_sets m
JOIN files_mode_shared sh ON sh.schedule_id = m.schedule_id AND NOT sh.file_change_patterns;

-- Commands.
UPDATE schedules s
SET pre_backup_commands = m.pre_backup_commands,
    post_backup_commands = m.post_backup_commands
FROM files_mode_sets m
JOIN files_mode_shared sh ON sh.schedule_id = m.schedule_id AND sh.commands
WHERE s.id = m.schedule_id;

DELETE FROM per_agent_commands
WHERE schedule_id IN (SELECT schedule_id FROM files_mode_shared);

INSERT INTO per_agent_commands (schedule_id, agent_id, pre_backup_commands, post_backup_commands)
SELECT m.schedule_id, m.agent_id, m.pre_backup_commands, m.post_backup_commands
FROM files_mode_sets m
JOIN files_mode_shared sh ON sh.schedule_id = m.schedule_id AND NOT sh.commands;
