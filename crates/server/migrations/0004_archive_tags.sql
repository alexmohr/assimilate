-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

CREATE TABLE archive_tags (
    id BIGSERIAL PRIMARY KEY,
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    archive_name TEXT NOT NULL,
    tag TEXT NOT NULL,
    created_by BIGINT REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(repo_id, archive_name, tag)
);

CREATE INDEX idx_archive_tags_repo_archive ON archive_tags (repo_id, archive_name);
CREATE INDEX idx_archive_tags_tag ON archive_tags (tag);
