-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- A dependency host is a machine a backup needs besides its agent and its
-- repository: the SMB or NFS server whose share a pre-backup command mounts,
-- say. Assimilate checks that it answers on a TCP port before the backup runs,
-- can wake it, and - for one marked as not always online - waits for it the
-- way it already waits for agents and repository hosts.
--
-- A dependency that is the same machine as a repository host points at it with
-- repo_host_id and takes that host's wake settings instead of its own, so the
-- machine has one MAC address and one shut-down switch. Its own wake columns
-- are then ignored. ON DELETE SET NULL rather than CASCADE: removing the
-- repository host leaves the dependency in place, falling back to its own
-- (possibly empty) wake settings.
CREATE TABLE dependency_hosts (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE CHECK (name <> ''),
    address TEXT NOT NULL CHECK (address <> ''),
    port INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    description TEXT NOT NULL DEFAULT '',
    repo_host_id BIGINT REFERENCES repo_hosts(id) ON DELETE SET NULL,
    wake_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    wake_mac_address TEXT,
    wake_broadcast_address TEXT,
    wake_timeout_seconds INTEGER NOT NULL DEFAULT 180 CHECK (wake_timeout_seconds > 0),
    intermittent BOOLEAN NOT NULL DEFAULT FALSE,
    catch_up_recheck_minutes INTEGER NOT NULL DEFAULT 15 CHECK (catch_up_recheck_minutes > 0),
    catch_up_give_up_minutes INTEGER NOT NULL DEFAULT 0 CHECK (catch_up_give_up_minutes >= 0),
    -- The most recent answer to "is it up", from a run, the poller, Check now
    -- or Test connection - what the list shows without probing on every load.
    last_checked_at TIMESTAMPTZ,
    last_check_reachable BOOLEAN,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT dependency_hosts_wake_mac_format
        CHECK (wake_mac_address IS NULL
            OR wake_mac_address ~* '^([0-9A-F]{2}:){5}[0-9A-F]{2}$'),
    CONSTRAINT dependency_hosts_wake_requires_mac
        CHECK (NOT wake_enabled OR wake_mac_address IS NOT NULL)
);

CREATE INDEX idx_dependency_hosts_repo_host_id ON dependency_hosts (repo_host_id);

-- Which dependencies one agent needs when it runs one schedule. Per agent,
-- because each agent mounts its own shares: the same schedule may need
-- nas-media on one host and nothing on another.
CREATE TABLE schedule_dependencies (
    schedule_id BIGINT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    agent_id BIGINT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    dependency_host_id BIGINT NOT NULL REFERENCES dependency_hosts(id) ON DELETE CASCADE,
    PRIMARY KEY (schedule_id, agent_id, dependency_host_id)
);

CREATE INDEX idx_schedule_dependencies_dependency ON schedule_dependencies (dependency_host_id);

-- Dependencies every schedule on an agent needs, for a share the agent's own
-- default pre-backup commands mount. Added to whatever a schedule sets itself.
CREATE TABLE agent_default_dependencies (
    agent_id BIGINT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    dependency_host_id BIGINT NOT NULL REFERENCES dependency_hosts(id) ON DELETE CASCADE,
    PRIMARY KEY (agent_id, dependency_host_id)
);

CREATE INDEX idx_agent_default_dependencies_dependency
    ON agent_default_dependencies (dependency_host_id);

-- One target that skipped a run because a dependency marked as not always
-- online did not answer. Per (schedule, agent), matching
-- schedule_targets.catch_up_pending_for: a skipped target skipped every
-- repository it would have written. dependency_host_id is the one that was
-- away and is asked on its re-check interval.
--
-- The row stays until the target actually runs: dispatched_run_id marks the
-- catch-up handed to it, and the agent's report for the schedule clears the
-- row. A catch-up that finds a dependency away again only releases
-- dispatched_run_id, so retries keep the original occurrence and never push
-- the give-up window forward. A later scheduled miss overwrites pending_for,
-- so however many occurrences pass, at most one catch-up follows.
CREATE TABLE dependency_catch_ups (
    schedule_id BIGINT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    agent_id BIGINT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    dependency_host_id BIGINT NOT NULL REFERENCES dependency_hosts(id) ON DELETE CASCADE,
    pending_for TIMESTAMPTZ NOT NULL,
    last_probe_at TIMESTAMPTZ,
    dispatched_run_id TEXT,
    PRIMARY KEY (schedule_id, agent_id)
);

CREATE INDEX idx_dependency_catch_ups_dependency ON dependency_catch_ups (dependency_host_id);

-- A run a dependency kept from happening is recorded as a report of its own,
-- so it shows in the run history with the reason, rather than leaving the
-- 'pending' row the tick inserted to be replayed on the agent's next
-- reconnect. Like 'cancelled' it is not an outcome: nothing ran.
ALTER TABLE backup_reports DROP CONSTRAINT backup_reports_status_check;

ALTER TABLE backup_reports
    ADD CONSTRAINT backup_reports_status_check
    CHECK (status IN (
        'pending', 'started', 'cancelled', 'skipped', 'success', 'warning', 'failed'
    ));

-- The run timeline gains a third kind of host, and a step for one that never
-- answered: a dependency that is still closed after its wake is what decides
-- the run, so it is said on the timeline rather than left implied.
ALTER TABLE backup_run_events DROP CONSTRAINT backup_run_events_target_check;

ALTER TABLE backup_run_events
    ADD CONSTRAINT backup_run_events_target_check
    CHECK (target IN ('source', 'repository', 'dependency')) NOT VALID;

ALTER TABLE backup_run_events DROP CONSTRAINT backup_run_events_event_type_check;

ALTER TABLE backup_run_events
    ADD CONSTRAINT backup_run_events_event_type_check
    CHECK (event_type IN (
        'reachability_check', 'wake_sent', 'wake_unavailable', 'host_online',
        'host_unreachable', 'agent_start_sent', 'agent_connected',
        'agent_stop_sent', 'agent_stopped', 'shutdown_sent', 'host_offline'
    )) NOT VALID;

-- shared::types::SystemEventType gained two variants, the dependency
-- counterparts of the agent-offline pair:
--
-- * backup_skipped_dependency_offline: a dependency marked as not always
--   online did not answer, so the run was skipped and will be caught up.
-- * backup_failed_dependency_offline: one that should always be there did not
--   answer. No agent ever ran, so this is the Activity Log's only record.
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
        'backup_skipped_dependency_offline', 'backup_failed_dependency_offline'
    )) NOT VALID;

-- notifications::EventType gained backup_skipped_dependency_offline. New, so
-- no existing rule is twinned onto it: a channel opts in. A dependency that
-- should always be there and does not answer is a plain backup_failed, which
-- existing rules already cover, and one that never comes back is
-- backup_catch_up_abandoned.
ALTER TABLE notification_rules DROP CONSTRAINT notification_rules_event_type_check;

ALTER TABLE notification_rules
    ADD CONSTRAINT notification_rules_event_type_check
    CHECK (event_type IN (
        'backup_success', 'backup_warning', 'backup_failed',
        'check_success', 'check_failed',
        'agent_connected', 'agent_disconnected',
        'schedule_auto_disabled', 'backup_skipped_agent_offline',
        'backup_skipped_repo_offline', 'backup_file_changed',
        'backup_catch_up_abandoned', 'backup_skipped_dependency_offline'
    )) NOT VALID;
