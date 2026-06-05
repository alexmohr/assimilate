-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

CREATE TABLE archive_files (
    id           BIGSERIAL PRIMARY KEY,
    repo_id      BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    archive_name TEXT   NOT NULL,
    path         TEXT   NOT NULL,
    parent_path  TEXT   NOT NULL,
    entry_type   TEXT   NOT NULL,
    size         BIGINT NOT NULL DEFAULT 0,
    mtime        TEXT   NOT NULL DEFAULT '',
    mode         TEXT   NOT NULL DEFAULT '',
    UNIQUE (repo_id, archive_name, path)
);

-- Fast directory listing: WHERE repo_id=$1 AND archive_name=$2 AND parent_path=$3
CREATE INDEX idx_archive_files_dir ON archive_files (repo_id, archive_name, parent_path);

CREATE TABLE archive_index_jobs (
    repo_id       BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    archive_name  TEXT   NOT NULL,
    status        TEXT   NOT NULL DEFAULT 'pending',
    started_at    TIMESTAMPTZ,
    finished_at   TIMESTAMPTZ,
    file_count    BIGINT,
    error_message TEXT,
    PRIMARY KEY (repo_id, archive_name)
);
