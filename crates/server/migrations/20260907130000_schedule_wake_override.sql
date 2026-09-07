-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Lets a single schedule decide for itself whether the hosts it needs are
-- woken, instead of that being settled once per host for every schedule that
-- touches it. 'host_default' leaves the host's own wake_enabled flag as the
-- answer (what every existing schedule does); 'enabled' wakes even where the
-- host has waking switched off; 'disabled' never wakes, whatever the host
-- says. Only waking is overridden -- starting the agent process over SSH
-- still follows the agent's own start_agent_enabled flag, and a host is
-- still only shut down where this run is what woke it (see
-- crates/server/src/power.rs).
ALTER TABLE schedules
    ADD COLUMN wake_override TEXT NOT NULL DEFAULT 'host_default'
        CHECK (wake_override IN ('host_default', 'enabled', 'disabled'));

-- A host whose only waker is a per-schedule override still has to be allowed
-- a shutdown, so both shutdown constraints key off having something to wake
-- (a MAC address) instead of off the host's own wake toggle. Every row that
-- satisfied the old constraint satisfies this one: wake_enabled already
-- requires a MAC address (agents_wake_requires_mac / repos_wake_requires_mac).
ALTER TABLE agents DROP CONSTRAINT agents_shutdown_requires_wake;
ALTER TABLE agents
    ADD CONSTRAINT agents_shutdown_requires_mac
    CHECK (NOT shutdown_after_backup OR wake_mac_address IS NOT NULL);

ALTER TABLE repos DROP CONSTRAINT repos_shutdown_requires_wake;
ALTER TABLE repos
    ADD CONSTRAINT repos_shutdown_requires_mac
    CHECK (NOT shutdown_after_backup OR wake_mac_address IS NOT NULL);

-- shared::types::RunEventType gained WakeUnavailable: a schedule can now ask
-- for a wake on a host that has no MAC address on file, which is recorded on
-- the run timeline rather than passing silently. Extend the closed set the
-- backup_run_events migration locked event_type to, the same way
-- 20260813170000_archive_compact_failed_event_type.sql did for system_events.
ALTER TABLE backup_run_events DROP CONSTRAINT backup_run_events_event_type_check;

ALTER TABLE backup_run_events
    ADD CONSTRAINT backup_run_events_event_type_check
    CHECK (event_type IN (
        'reachability_check', 'wake_sent', 'wake_unavailable', 'host_online',
        'agent_start_sent', 'agent_connected',
        'agent_stop_sent', 'agent_stopped', 'shutdown_sent', 'host_offline'
    )) NOT VALID;
