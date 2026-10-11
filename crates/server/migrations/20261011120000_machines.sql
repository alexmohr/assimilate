-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- A machine is what is woken, shut down, and expected to be online or not.
-- Agents, repository hosts and dependency hosts used to carry those facts each
-- in their own columns, so one physical box had as many copies of its MAC
-- address and its "not always online" switch as roles it played. They are now
-- roles of a machine: each points at one, and keeps only what is its own (an
-- agent's process and SSH settings, a repository host's SSH host key, a
-- dependency's checked port).
--
-- Machines are never merged by MAC address: two rows that share one could be
-- a typo as easily as one box, and a wrong merge is hard to undo. The only
-- machines shared from the start are the ones already declared to be one: a
-- dependency marked as the same machine as a repository host.
CREATE TABLE machines (
    id BIGSERIAL PRIMARY KEY,
    wake_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    wake_mac_address TEXT,
    wake_broadcast_address TEXT,
    wake_timeout_seconds INTEGER NOT NULL DEFAULT 180 CHECK (wake_timeout_seconds > 0),
    shutdown_after_backup BOOLEAN NOT NULL DEFAULT FALSE,
    intermittent BOOLEAN NOT NULL DEFAULT FALSE,
    catch_up_recheck_minutes INTEGER NOT NULL DEFAULT 15 CHECK (catch_up_recheck_minutes > 0),
    catch_up_give_up_minutes INTEGER NOT NULL DEFAULT 0 CHECK (catch_up_give_up_minutes >= 0),
    -- The most recent answer to "is it up", from a run, a poller or a manual
    -- check - what a list shows without probing on every load.
    last_checked_at TIMESTAMPTZ,
    last_check_reachable BOOLEAN,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Only used while this migration runs, to find each new row again.
    legacy_agent_id BIGINT,
    legacy_repo_host_id BIGINT,
    legacy_dependency_host_id BIGINT,
    CONSTRAINT machines_wake_mac_format
        CHECK (wake_mac_address IS NULL
            OR wake_mac_address ~* '^([0-9A-F]{2}:){5}[0-9A-F]{2}$'),
    CONSTRAINT machines_wake_requires_mac
        CHECK (NOT wake_enabled OR wake_mac_address IS NOT NULL),
    CONSTRAINT machines_shutdown_requires_mac
        CHECK (NOT shutdown_after_backup OR wake_mac_address IS NOT NULL)
);

-- The event types come first: the merge below writes to the Activity Log.
--
-- shared::types::SystemEventType gained two variants:
--
-- * backup_waiting_for_host: a schedule set to wait for a machine found it
--   down, so the target was skipped and will be caught up.
-- * machine_migrated: what this migration decided when it merged a dependency
--   into its repository host's machine.
ALTER TABLE system_events DROP CONSTRAINT system_events_event_type_check;

ALTER TABLE system_events
    ADD CONSTRAINT system_events_event_type_check
    CHECK (event_type IN (
        'repo_sync', 'repo_sync_slow', 'repo_sync_failed', 'repo_sync_cancelled',
        'archive_delete_failed', 'archive_compact_failed', 'account_locked',
        'auth_failed', 'security_violation', 'schedule_auto_disabled',
        'schedule_reenabled', 'schedule_catch_up', 'backup_skipped_agent_offline',
        'backup_skipped_repo_offline', 'schedule_catch_up_abandoned',
        'backup_failed_agent_offline', 'repo_host_migrated',
        'backup_skipped_dependency_offline', 'backup_failed_dependency_offline',
        'backup_waiting_for_host', 'machine_migrated'
    )) NOT VALID;

-- Agents. An agent never had a re-check interval (it reconnects on its own),
-- so it takes the default, which is only used if the machine ever gains
-- another role.
INSERT INTO machines (
    wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds,
    shutdown_after_backup, intermittent, catch_up_give_up_minutes, legacy_agent_id
)
SELECT wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds,
    shutdown_after_backup, intermittent, catch_up_give_up_minutes, id
FROM agents;

ALTER TABLE agents ADD COLUMN machine_id BIGINT REFERENCES machines(id);

UPDATE agents a SET machine_id = m.id FROM machines m WHERE m.legacy_agent_id = a.id;

-- Repository hosts.
INSERT INTO machines (
    wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds,
    shutdown_after_backup, intermittent, catch_up_recheck_minutes, catch_up_give_up_minutes,
    legacy_repo_host_id
)
SELECT wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds,
    shutdown_after_backup, intermittent, catch_up_recheck_minutes, catch_up_give_up_minutes, id
FROM repo_hosts;

ALTER TABLE repo_hosts ADD COLUMN machine_id BIGINT REFERENCES machines(id);

UPDATE repo_hosts h SET machine_id = m.id FROM machines m WHERE m.legacy_repo_host_id = h.id;

-- Dependencies on their own machine.
INSERT INTO machines (
    wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds,
    intermittent, catch_up_recheck_minutes, catch_up_give_up_minutes, last_checked_at,
    last_check_reachable, legacy_dependency_host_id
)
SELECT wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds,
    intermittent, catch_up_recheck_minutes, catch_up_give_up_minutes, last_checked_at,
    last_check_reachable, id
FROM dependency_hosts
WHERE repo_host_id IS NULL;

ALTER TABLE dependency_hosts ADD COLUMN machine_id BIGINT REFERENCES machines(id);

UPDATE dependency_hosts d SET machine_id = m.id
FROM machines m WHERE m.legacy_dependency_host_id = d.id;

-- A dependency that is the same machine as a repository host already used
-- that host's wake settings; it now shares the whole machine. Where the two
-- disagreed on whether the machine is always online, the merge keeps what
-- changes nothing for the worse, the same rules the repository host migration
-- used: not always online if either said so, the shorter re-check interval,
-- the longer give-up window with "wait indefinitely" winning. Each difference
-- is written to the Activity Log first, while both values are still there.
INSERT INTO system_events (event_type, hostname, message)
SELECT 'machine_migrated', h.ssh_host,
    format(
        'Dependency ''%s'' is the same machine as repository host %s and now shares its '
        || 'settings. %s',
        d.name, h.ssh_host,
        concat_ws(' ',
            CASE WHEN d.intermittent <> h.intermittent THEN
                'One of them was marked as not always online, so the machine now is.'
            END,
            CASE WHEN d.catch_up_recheck_minutes <> h.catch_up_recheck_minutes THEN
                format('It is re-checked every %s minutes, the shorter interval.',
                    LEAST(d.catch_up_recheck_minutes, h.catch_up_recheck_minutes))
            END,
            CASE WHEN d.catch_up_give_up_minutes <> h.catch_up_give_up_minutes THEN
                'Runs waiting on it keep the longer give-up window.'
            END,
            CASE WHEN d.wake_enabled OR d.wake_mac_address IS NOT NULL THEN
                'The dependency''s own wake settings, unused while it shared the host''s, '
                || 'were removed.'
            END
        )
    )
FROM dependency_hosts d
JOIN repo_hosts h ON h.id = d.repo_host_id
WHERE d.intermittent <> h.intermittent
    OR d.catch_up_recheck_minutes <> h.catch_up_recheck_minutes
    OR d.catch_up_give_up_minutes <> h.catch_up_give_up_minutes
    OR d.wake_enabled
    OR d.wake_mac_address IS NOT NULL;

UPDATE machines m SET
    intermittent = m.intermittent OR d.intermittent,
    catch_up_recheck_minutes = LEAST(m.catch_up_recheck_minutes, d.catch_up_recheck_minutes),
    catch_up_give_up_minutes = CASE
        WHEN m.catch_up_give_up_minutes = 0 OR d.catch_up_give_up_minutes = 0 THEN 0
        ELSE GREATEST(m.catch_up_give_up_minutes, d.catch_up_give_up_minutes)
    END,
    last_checked_at = d.last_checked_at,
    last_check_reachable = d.last_check_reachable
FROM dependency_hosts d
JOIN repo_hosts h ON h.id = d.repo_host_id
WHERE m.id = h.machine_id;

UPDATE dependency_hosts d SET machine_id = h.machine_id
FROM repo_hosts h WHERE h.id = d.repo_host_id;

ALTER TABLE machines
    DROP COLUMN legacy_agent_id,
    DROP COLUMN legacy_repo_host_id,
    DROP COLUMN legacy_dependency_host_id;

ALTER TABLE agents ALTER COLUMN machine_id SET NOT NULL;
ALTER TABLE repo_hosts ALTER COLUMN machine_id SET NOT NULL;
ALTER TABLE dependency_hosts ALTER COLUMN machine_id SET NOT NULL;

CREATE INDEX idx_agents_machine_id ON agents (machine_id);
CREATE INDEX idx_repo_hosts_machine_id ON repo_hosts (machine_id);
CREATE INDEX idx_dependency_hosts_machine_id ON dependency_hosts (machine_id);

-- One machine per agent and per repository host. Only dependencies share.
ALTER TABLE agents ADD CONSTRAINT agents_machine_id_key UNIQUE (machine_id);
ALTER TABLE repo_hosts ADD CONSTRAINT repo_hosts_machine_id_key UNIQUE (machine_id);

ALTER TABLE agents
    DROP CONSTRAINT agents_wake_requires_mac,
    DROP CONSTRAINT agents_wake_mac_format,
    DROP CONSTRAINT agents_shutdown_requires_mac,
    DROP CONSTRAINT agents_shutdown_requires_ssh_host,
    DROP COLUMN wake_enabled,
    DROP COLUMN wake_mac_address,
    DROP COLUMN wake_broadcast_address,
    DROP COLUMN wake_timeout_seconds,
    DROP COLUMN shutdown_after_backup,
    DROP COLUMN intermittent,
    DROP COLUMN catch_up_give_up_minutes;

ALTER TABLE repo_hosts
    DROP CONSTRAINT repo_hosts_wake_mac_format,
    DROP CONSTRAINT repo_hosts_wake_requires_mac,
    DROP CONSTRAINT repo_hosts_shutdown_requires_mac,
    DROP COLUMN wake_enabled,
    DROP COLUMN wake_mac_address,
    DROP COLUMN wake_broadcast_address,
    DROP COLUMN wake_timeout_seconds,
    DROP COLUMN shutdown_after_backup,
    DROP COLUMN intermittent,
    DROP COLUMN catch_up_recheck_minutes,
    DROP COLUMN catch_up_give_up_minutes;

-- "Same machine as" is now simply a shared machine_id.
DROP INDEX idx_dependency_hosts_repo_host_id;

ALTER TABLE dependency_hosts
    DROP CONSTRAINT dependency_hosts_wake_mac_format,
    DROP CONSTRAINT dependency_hosts_wake_requires_mac,
    DROP COLUMN repo_host_id,
    DROP COLUMN wake_enabled,
    DROP COLUMN wake_mac_address,
    DROP COLUMN wake_broadcast_address,
    DROP COLUMN wake_timeout_seconds,
    DROP COLUMN intermittent,
    DROP COLUMN catch_up_recheck_minutes,
    DROP COLUMN catch_up_give_up_minutes,
    DROP COLUMN last_checked_at,
    DROP COLUMN last_check_reachable;

-- A role inserted without a machine gets a fresh one with the defaults, so
-- every path that creates an agent (registration, an import placeholder, a
-- merge), a repository host or a dependency has one without having to say so.
CREATE FUNCTION assign_new_machine() RETURNS trigger AS $$
BEGIN
    IF NEW.machine_id IS NULL THEN
        INSERT INTO machines DEFAULT VALUES RETURNING id INTO NEW.machine_id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER agents_assign_new_machine
    BEFORE INSERT ON agents
    FOR EACH ROW EXECUTE FUNCTION assign_new_machine();

CREATE TRIGGER repo_hosts_assign_new_machine
    BEFORE INSERT ON repo_hosts
    FOR EACH ROW EXECUTE FUNCTION assign_new_machine();

CREATE TRIGGER dependency_hosts_assign_new_machine
    BEFORE INSERT ON dependency_hosts
    FOR EACH ROW EXECUTE FUNCTION assign_new_machine();

-- A machine no role points at any more is removed with the last of them, so
-- deleting an agent, a repository host or a dependency never leaves a MAC
-- address behind.
CREATE FUNCTION delete_orphaned_machine() RETURNS trigger AS $$
BEGIN
    DELETE FROM machines m
    WHERE m.id = OLD.machine_id
        AND NOT EXISTS (SELECT 1 FROM agents WHERE machine_id = m.id)
        AND NOT EXISTS (SELECT 1 FROM repo_hosts WHERE machine_id = m.id)
        AND NOT EXISTS (SELECT 1 FROM dependency_hosts WHERE machine_id = m.id);
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER agents_delete_orphaned_machine
    AFTER DELETE OR UPDATE OF machine_id ON agents
    FOR EACH ROW EXECUTE FUNCTION delete_orphaned_machine();

CREATE TRIGGER repo_hosts_delete_orphaned_machine
    AFTER DELETE OR UPDATE OF machine_id ON repo_hosts
    FOR EACH ROW EXECUTE FUNCTION delete_orphaned_machine();

CREATE TRIGGER dependency_hosts_delete_orphaned_machine
    AFTER DELETE OR UPDATE OF machine_id ON dependency_hosts
    FOR EACH ROW EXECUTE FUNCTION delete_orphaned_machine();

-- What one schedule does about one machine, where it differs from the
-- schedule-wide wake_override:
--
-- * wake:    wake it if it does not answer, whatever the machine says.
-- * no_wake: never wake it; it has to be up already.
-- * wait:    never wake it, and if it does not answer, skip the target and
--            catch it up once it does - as if the machine were marked as not
--            always online, for this schedule only.
--
-- No row means the schedule-wide setting applies.
CREATE TABLE schedule_machine_wake (
    schedule_id BIGINT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    machine_id BIGINT NOT NULL REFERENCES machines(id) ON DELETE CASCADE,
    mode TEXT NOT NULL CHECK (mode IN ('wake', 'no_wake', 'wait')),
    PRIMARY KEY (schedule_id, machine_id)
);

CREATE INDEX idx_schedule_machine_wake_machine ON schedule_machine_wake (machine_id);

-- Dependencies every backup written to a repository host needs - a power
-- switch in front of the NAS, say. Added to what the schedule and the agent's
-- defaults require, for the targets that write to that host.
CREATE TABLE repo_host_dependencies (
    repo_host_id BIGINT NOT NULL REFERENCES repo_hosts(id) ON DELETE CASCADE,
    dependency_host_id BIGINT NOT NULL REFERENCES dependency_hosts(id) ON DELETE CASCADE,
    PRIMARY KEY (repo_host_id, dependency_host_id)
);

CREATE INDEX idx_repo_host_dependencies_dependency
    ON repo_host_dependencies (dependency_host_id);

-- notifications::EventType gained backup_waiting_for_host. New, so no
-- existing rule is twinned onto it: a schedule that waits on purpose does not
-- alert like an outage unless a channel opts in.
ALTER TABLE notification_rules DROP CONSTRAINT notification_rules_event_type_check;

ALTER TABLE notification_rules
    ADD CONSTRAINT notification_rules_event_type_check
    CHECK (event_type IN (
        'backup_success', 'backup_warning', 'backup_failed',
        'check_success', 'check_failed',
        'agent_connected', 'agent_disconnected',
        'schedule_auto_disabled', 'backup_skipped_agent_offline',
        'backup_skipped_repo_offline', 'backup_file_changed',
        'backup_catch_up_abandoned', 'backup_skipped_dependency_offline',
        'backup_waiting_for_host'
    )) NOT VALID;
