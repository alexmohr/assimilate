-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Normalize the archive content index tables for storage efficiency.
--
-- Before: archive_name TEXT repeated in archive_paths/archive_files/archive_index_jobs, and
-- paths were deduplicated only per-archive. A repo with 30 daily archives stored the same
-- tree 30 times — the index was 91.7% of total DB size (10+ GB).
--
-- After:
--   archives         — one row per (repo_id, archive_name); normalizes the TEXT name to BIGINT id
--   archive_paths    — one row per (repo_id, path); shared across all archives in a repo
--   archive_files    — one row per (archive_id, path_id); references both foreign keys by BIGINT
--   archive_index_jobs — keyed by archive_id instead of (repo_id, archive_name)
--   archive_tags     — references archive_id instead of storing (repo_id, archive_name) TEXT
--
-- Content index is rebuilt lazily on next browse; tags (user data) are preserved via FK migration.

-- Step 1: normalized archive name table
CREATE TABLE archives (
    id      BIGSERIAL PRIMARY KEY,
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    name    TEXT NOT NULL,
    UNIQUE (repo_id, name)
);

-- Populate from all known archive names across the system (tags take precedence as user data).
INSERT INTO archives (repo_id, name)
SELECT DISTINCT repo_id, archive_name
FROM archive_tags
ON CONFLICT DO NOTHING;

INSERT INTO archives (repo_id, name)
SELECT DISTINCT repo_id, archive_name
FROM backup_reports
WHERE archive_name IS NOT NULL
ON CONFLICT DO NOTHING;

-- Step 2: migrate archive_tags to reference archive_id
ALTER TABLE archive_tags ADD COLUMN archive_id BIGINT REFERENCES archives(id) ON DELETE CASCADE;

UPDATE archive_tags t
SET archive_id = a.id
FROM archives a
WHERE a.repo_id = t.repo_id AND a.name = t.archive_name;

-- Discard any orphaned tag rows (archive_name referenced a repo that no longer has that archive)
DELETE FROM archive_tags WHERE archive_id IS NULL;

ALTER TABLE archive_tags ALTER COLUMN archive_id SET NOT NULL;

DROP INDEX IF EXISTS idx_archive_tags_repo_archive;
ALTER TABLE archive_tags DROP CONSTRAINT archive_tags_repo_id_archive_name_tag_key;
ALTER TABLE archive_tags DROP COLUMN repo_id;
ALTER TABLE archive_tags DROP COLUMN archive_name;

ALTER TABLE archive_tags ADD CONSTRAINT archive_tags_archive_id_tag_key UNIQUE (archive_id, tag);
CREATE INDEX idx_archive_tags_archive ON archive_tags (archive_id);

-- Step 3: drop the old content index tables (rebuilt lazily from borg).
DROP TABLE IF EXISTS archive_files;
DROP TABLE IF EXISTS archive_paths;
DROP TABLE IF EXISTS archive_index_jobs;

-- Step 4: recreate content index tables with normalized schema.
CREATE TABLE archive_paths (
    id      BIGSERIAL PRIMARY KEY,
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    path    TEXT NOT NULL,
    UNIQUE (repo_id, path)
);

CREATE TABLE archive_files (
    id             BIGSERIAL PRIMARY KEY,
    archive_id     BIGINT NOT NULL REFERENCES archives(id) ON DELETE CASCADE,
    path_id        BIGINT NOT NULL REFERENCES archive_paths(id),
    parent_path_id BIGINT NOT NULL REFERENCES archive_paths(id),
    entry_type     TEXT NOT NULL,
    size           BIGINT NOT NULL DEFAULT 0,
    mtime          TEXT NOT NULL DEFAULT '',
    mode           TEXT NOT NULL DEFAULT '',
    UNIQUE (archive_id, path_id)
);

CREATE INDEX idx_archive_files_dir ON archive_files (archive_id, parent_path_id);

CREATE TABLE archive_index_jobs (
    archive_id    BIGINT NOT NULL PRIMARY KEY REFERENCES archives(id) ON DELETE CASCADE,
    status        TEXT NOT NULL DEFAULT 'pending',
    started_at    TIMESTAMPTZ,
    finished_at   TIMESTAMPTZ,
    file_count    BIGINT,
    error_message TEXT
);
