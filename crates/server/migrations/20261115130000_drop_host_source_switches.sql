-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- A host no longer turns virtual machine backups or database dumps on or off
-- as a whole: it backs up the domains and database sources it includes, and
-- each schedule takes them or opts out with its own switch.
--
-- A host that had either switched off backed up none of them, whatever its
-- schedules asked for. It keeps doing so: its selection becomes "only the
-- selected ones" with none selected. Everything it listed stays listed, so
-- including one again is a single click.
UPDATE agent_vms v
SET included = FALSE
FROM agents a
WHERE a.id = v.agent_id AND NOT a.vm_snapshot_enabled;

UPDATE agents SET vm_snapshot_selection = 'selected' WHERE NOT vm_snapshot_enabled;

UPDATE agent_database_sources s
SET included = FALSE
FROM agents a
WHERE a.id = s.agent_id AND NOT a.database_backup_enabled;

UPDATE agents SET database_selection = 'selected' WHERE NOT database_backup_enabled;

ALTER TABLE agents
    DROP COLUMN vm_snapshot_enabled,
    DROP COLUMN database_backup_enabled;
