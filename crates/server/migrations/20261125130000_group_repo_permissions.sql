-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Per-repository permissions granted to a group: every member holds them, on
-- top of their own grants in repo_permissions.
CREATE TABLE group_repo_permissions (
    group_id BIGINT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    can_view BOOLEAN NOT NULL DEFAULT false,
    can_backup BOOLEAN NOT NULL DEFAULT false,
    can_modify_schedules BOOLEAN NOT NULL DEFAULT false,
    can_extract BOOLEAN NOT NULL DEFAULT false,
    can_delete BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY (group_id, repo_id)
);

CREATE INDEX idx_group_repo_permissions_repo_id ON group_repo_permissions(repo_id);
