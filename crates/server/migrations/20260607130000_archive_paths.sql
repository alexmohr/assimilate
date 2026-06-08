-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

CREATE TABLE archive_paths (
    id           BIGSERIAL PRIMARY KEY,
    repo_id      BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    archive_name TEXT   NOT NULL,
    path         TEXT   NOT NULL,
    UNIQUE (repo_id, archive_name, path)
);

ALTER TABLE archive_files
    ADD COLUMN path_id BIGINT,
    ADD COLUMN parent_path_id BIGINT;

INSERT INTO archive_paths (repo_id, archive_name, path)
SELECT DISTINCT repo_id, archive_name, path
FROM (
    SELECT repo_id, archive_name, path FROM archive_files
    UNION ALL
    SELECT repo_id, archive_name, parent_path AS path FROM archive_files
) archive_paths_source
ON CONFLICT DO NOTHING;

UPDATE archive_files files
SET path_id = paths.id
FROM archive_paths paths
WHERE paths.repo_id = files.repo_id
    AND paths.archive_name = files.archive_name
    AND paths.path = files.path;

UPDATE archive_files files
SET parent_path_id = paths.id
FROM archive_paths paths
WHERE paths.repo_id = files.repo_id
    AND paths.archive_name = files.archive_name
    AND paths.path = files.parent_path;

ALTER TABLE archive_files
    ALTER COLUMN path_id SET NOT NULL,
    ALTER COLUMN parent_path_id SET NOT NULL;

ALTER TABLE archive_files
    ADD CONSTRAINT archive_files_path_id_fkey
    FOREIGN KEY (path_id) REFERENCES archive_paths(id) ON DELETE CASCADE,
    ADD CONSTRAINT archive_files_parent_path_id_fkey
    FOREIGN KEY (parent_path_id) REFERENCES archive_paths(id) ON DELETE CASCADE;

DROP INDEX IF EXISTS idx_archive_files_dir;
ALTER TABLE archive_files DROP CONSTRAINT IF EXISTS archive_files_repo_id_archive_name_path_key;
ALTER TABLE archive_files DROP COLUMN path;
ALTER TABLE archive_files DROP COLUMN parent_path;

CREATE UNIQUE INDEX archive_files_repo_id_archive_name_path_id_key
    ON archive_files (repo_id, archive_name, path_id);
CREATE INDEX idx_archive_files_dir ON archive_files (repo_id, archive_name, parent_path_id);
