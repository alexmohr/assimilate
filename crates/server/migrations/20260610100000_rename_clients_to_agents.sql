-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Rename clients table to agents
ALTER TABLE clients RENAME TO agents;

-- Rename host_tags table to agent_tags
ALTER TABLE host_tags RENAME TO agent_tags;

-- Rename client_hostname_patterns table to agent_hostname_patterns
ALTER TABLE client_hostname_patterns RENAME TO agent_hostname_patterns;

-- Rename client_id column to agent_id in all affected tables

-- backup_reports
ALTER TABLE backup_reports RENAME COLUMN client_id TO agent_id;

-- schedule_targets
ALTER TABLE schedule_targets RENAME COLUMN client_id TO agent_id;

-- agent_tags (formerly host_tags)
ALTER TABLE agent_tags RENAME COLUMN client_id TO agent_id;

-- ssh_tunnels
ALTER TABLE ssh_tunnels RENAME COLUMN client_id TO agent_id;

-- agent_hostname_patterns (formerly client_hostname_patterns)
ALTER TABLE agent_hostname_patterns RENAME COLUMN client_id TO agent_id;

-- backup_sources (added client_id in 20260531120000_per_host_backup_sources.sql)
ALTER TABLE backup_sources RENAME COLUMN client_id TO agent_id;

-- schedule_excludes (added client_id in 20260531140000_per_host_excludes.sql)
ALTER TABLE schedule_excludes RENAME COLUMN client_id TO agent_id;

-- notification_rules
ALTER TABLE notification_rules RENAME COLUMN client_id TO agent_id;

-- Update the tags CHECK constraint: change scope value 'host' -> 'agent'
ALTER TABLE tags DROP CONSTRAINT tags_scope_check;
ALTER TABLE tags ADD CONSTRAINT tags_scope_check CHECK (scope IN ('repo', 'agent'));
UPDATE tags SET scope = 'agent' WHERE scope = 'host';

-- Rename roles columns: can_create_client -> can_create_agent, etc.
ALTER TABLE roles RENAME COLUMN can_create_client TO can_create_agent;
ALTER TABLE roles RENAME COLUMN can_delete_client TO can_delete_agent;
ALTER TABLE roles RENAME COLUMN can_delete_own_client TO can_delete_own_agent;

-- Rename indexes

-- backup_reports
ALTER INDEX idx_backup_reports_client_id RENAME TO idx_backup_reports_agent_id;

-- schedule_targets
ALTER INDEX idx_schedule_targets_client_id RENAME TO idx_schedule_targets_agent_id;

-- agent_tags (formerly host_tags)
ALTER INDEX idx_host_tags_tag_id RENAME TO idx_agent_tags_tag_id;

-- agents (formerly clients)
ALTER INDEX idx_clients_is_hidden RENAME TO idx_agents_is_hidden;

-- agent_hostname_patterns (formerly client_hostname_patterns)
ALTER INDEX idx_client_hostname_patterns_pattern RENAME TO idx_agent_hostname_patterns_pattern;

-- Update the unique index on backup_sources that references the old column name
-- (the index backup_sources_schedule_client_path_idx uses COALESCE(client_id, -1))
DROP INDEX backup_sources_schedule_client_path_idx;
CREATE UNIQUE INDEX backup_sources_schedule_agent_path_idx
    ON backup_sources (schedule_id, COALESCE(agent_id, -1), path);

-- Update the unique index on schedule_excludes that references the old column name
DROP INDEX schedule_excludes_schedule_client_pattern_idx;
CREATE UNIQUE INDEX schedule_excludes_schedule_agent_pattern_idx
    ON schedule_excludes (schedule_id, COALESCE(agent_id, -1), pattern);
