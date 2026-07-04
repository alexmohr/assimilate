-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Extract derived statistics (borg cache snapshot) into its own 1:1 table.
CREATE TABLE repo_stats (
    repo_id BIGINT PRIMARY KEY REFERENCES repos(id) ON DELETE CASCADE,
    original_size BIGINT NOT NULL DEFAULT 0,
    compressed_size BIGINT NOT NULL DEFAULT 0,
    deduplicated_size BIGINT NOT NULL DEFAULT 0,
    total_chunks BIGINT NOT NULL DEFAULT 0,
    unique_chunks BIGINT NOT NULL DEFAULT 0,
    archive_count INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ,
    last_synced_at TIMESTAMPTZ
);

INSERT INTO repo_stats (repo_id, original_size, compressed_size, deduplicated_size, total_chunks, unique_chunks, archive_count, updated_at, last_synced_at)
SELECT id, info_original_size, info_compressed_size, info_deduplicated_size, info_total_chunks, info_unique_chunks, info_archive_count, info_updated_at, last_synced_at FROM repos;

-- Extract transient import state into its own 1:0/1 table.
CREATE TABLE repo_import_state (
    repo_id BIGINT PRIMARY KEY REFERENCES repos(id) ON DELETE CASCADE,
    importing BOOLEAN NOT NULL DEFAULT FALSE,
    error TEXT,
    progress INTEGER NOT NULL DEFAULT 0,
    total INTEGER NOT NULL DEFAULT 0,
    status_message TEXT
);

INSERT INTO repo_import_state (repo_id, importing, error, progress, total, status_message)
SELECT id, importing, import_error, import_progress, import_total, import_status_message FROM repos;

-- Extract transient last-op state into its own 1:0/1 table.
CREATE TABLE repo_last_op (
    repo_id BIGINT PRIMARY KEY REFERENCES repos(id) ON DELETE CASCADE,
    kind TEXT,
    at TIMESTAMPTZ,
    by_text TEXT
);

INSERT INTO repo_last_op (repo_id, kind, at, by_text)
SELECT id, last_op_kind, last_op_at, last_op_by FROM repos;

-- Drop the columns from repos now that the data lives in satellite tables.
ALTER TABLE repos
    DROP COLUMN info_original_size,
    DROP COLUMN info_compressed_size,
    DROP COLUMN info_deduplicated_size,
    DROP COLUMN info_total_chunks,
    DROP COLUMN info_unique_chunks,
    DROP COLUMN info_archive_count,
    DROP COLUMN info_updated_at,
    DROP COLUMN last_synced_at,
    DROP COLUMN importing,
    DROP COLUMN import_error,
    DROP COLUMN import_progress,
    DROP COLUMN import_total,
    DROP COLUMN import_status_message,
    DROP COLUMN last_op_kind,
    DROP COLUMN last_op_at,
    DROP COLUMN last_op_by;
