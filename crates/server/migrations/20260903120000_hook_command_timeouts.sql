-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Hook commands were stored as a JSONB array of plain strings. They are now
-- objects that may carry a per-command `timeout_seconds`, overriding the
-- schedule-wide `hook_timeout_seconds`. Normalise the rows written before that
-- so the stored shape matches what is written from now on; the Rust type still
-- reads the bare-string form, but only so an older export can be imported.

CREATE OR REPLACE FUNCTION assimilate_hook_commands_to_objects(commands JSONB)
RETURNS JSONB AS $$
    SELECT COALESCE(
        jsonb_agg(
            CASE
                WHEN jsonb_typeof(entry) = 'string'
                    THEN jsonb_build_object('command', entry, 'timeout_seconds', NULL)
                ELSE entry
            END
            ORDER BY position
        ),
        '[]'::JSONB
    )
    FROM jsonb_array_elements(commands) WITH ORDINALITY AS elements(entry, position);
$$ LANGUAGE SQL IMMUTABLE;

UPDATE schedules
SET pre_backup_commands = assimilate_hook_commands_to_objects(pre_backup_commands),
    post_backup_commands = assimilate_hook_commands_to_objects(post_backup_commands);

UPDATE agents
SET default_pre_backup_commands =
        assimilate_hook_commands_to_objects(default_pre_backup_commands),
    default_post_backup_commands =
        assimilate_hook_commands_to_objects(default_post_backup_commands);

UPDATE per_agent_commands
SET pre_backup_commands = assimilate_hook_commands_to_objects(pre_backup_commands),
    post_backup_commands = assimilate_hook_commands_to_objects(post_backup_commands);

DROP FUNCTION assimilate_hook_commands_to_objects(JSONB);
