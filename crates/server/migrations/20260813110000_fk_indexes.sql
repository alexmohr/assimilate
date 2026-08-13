-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Postgres does not auto-index foreign-key columns. These were left
-- unindexed, so parent DELETEs (repo/agent/schedule/user) do a sequential
-- scan of the child table to enforce ON DELETE CASCADE/SET NULL, and common
-- "children of X" lookups scan too.

CREATE INDEX idx_notification_rules_repo_id ON notification_rules(repo_id);
CREATE INDEX idx_notification_rules_agent_id ON notification_rules(agent_id);
CREATE INDEX idx_notification_rules_schedule_id ON notification_rules(schedule_id);

CREATE INDEX idx_backup_sources_repo_id ON backup_sources(repo_id);
CREATE INDEX idx_backup_sources_schedule_id ON backup_sources(schedule_id);
CREATE INDEX idx_backup_sources_agent_id ON backup_sources(agent_id);

CREATE INDEX idx_per_agent_excludes_schedule_id ON per_agent_excludes(schedule_id);
CREATE INDEX idx_per_agent_excludes_agent_id ON per_agent_excludes(agent_id);

-- per_agent_commands' PK is (schedule_id, agent_id), which already covers
-- schedule_id-leading lookups/cascades; only the agent_id-leading direction
-- (cascading from `agents`) is missing.
CREATE INDEX idx_per_agent_commands_agent_id ON per_agent_commands(agent_id);

CREATE INDEX idx_schedules_repo_id ON schedules(repo_id);

CREATE INDEX idx_canary_results_schedule_id ON canary_results(schedule_id);

CREATE INDEX idx_archive_tags_created_by ON archive_tags(created_by);

-- archive_files' unique index on (archive_id, path_id) leads with archive_id,
-- so it doesn't serve path_id-leading cascades from archive_paths.
CREATE INDEX idx_archive_files_path_id ON archive_files(path_id);

-- repo_permissions' PK is (user_id, repo_id), which doesn't serve
-- repo_id-leading cascades when a repo is deleted.
CREATE INDEX idx_repo_permissions_repo_id ON repo_permissions(repo_id);
