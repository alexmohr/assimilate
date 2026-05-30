-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Add scope filtering to notification channels
-- scope is a JSONB object: {"repo_ids": [...], "client_ids": [...], "schedule_ids": [...]}
-- Empty arrays or null means "all" (no filtering)
ALTER TABLE notification_channels ADD COLUMN scope JSONB NOT NULL DEFAULT '{}';

-- Add schedule_id to notification_rules for backward compat
ALTER TABLE notification_rules ADD COLUMN schedule_id BIGINT REFERENCES schedules(id) ON DELETE CASCADE;
