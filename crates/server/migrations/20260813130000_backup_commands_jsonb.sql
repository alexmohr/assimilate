-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Pre/post-backup command lists were hand-serialized JSON arrays stored in
-- plain TEXT columns, requiring the application to serde_json::to_string /
-- from_str on every write/read (and to silently swallow encode errors as
-- "[]"). The stored values are already valid JSON, so this is a direct cast.

ALTER TABLE schedules
    ALTER COLUMN pre_backup_commands DROP DEFAULT,
    ALTER COLUMN pre_backup_commands TYPE JSONB USING pre_backup_commands::JSONB,
    ALTER COLUMN pre_backup_commands SET DEFAULT '[]'::JSONB,
    ALTER COLUMN post_backup_commands DROP DEFAULT,
    ALTER COLUMN post_backup_commands TYPE JSONB USING post_backup_commands::JSONB,
    ALTER COLUMN post_backup_commands SET DEFAULT '[]'::JSONB;

ALTER TABLE agents
    ALTER COLUMN default_pre_backup_commands DROP DEFAULT,
    ALTER COLUMN default_pre_backup_commands TYPE JSONB USING default_pre_backup_commands::JSONB,
    ALTER COLUMN default_pre_backup_commands SET DEFAULT '[]'::JSONB,
    ALTER COLUMN default_post_backup_commands DROP DEFAULT,
    ALTER COLUMN default_post_backup_commands TYPE JSONB USING default_post_backup_commands::JSONB,
    ALTER COLUMN default_post_backup_commands SET DEFAULT '[]'::JSONB;

ALTER TABLE per_agent_commands
    ALTER COLUMN pre_backup_commands DROP DEFAULT,
    ALTER COLUMN pre_backup_commands TYPE JSONB USING pre_backup_commands::JSONB,
    ALTER COLUMN pre_backup_commands SET DEFAULT '[]'::JSONB,
    ALTER COLUMN post_backup_commands DROP DEFAULT,
    ALTER COLUMN post_backup_commands TYPE JSONB USING post_backup_commands::JSONB,
    ALTER COLUMN post_backup_commands SET DEFAULT '[]'::JSONB;
