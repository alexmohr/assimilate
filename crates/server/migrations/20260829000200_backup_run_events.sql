-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- A granular, timestamped log of what a scheduled run's power management did
-- to reach its source and repository hosts (reachability checks, wake
-- packets, agent start/stop, shutdowns) and why -- shown as the run detail
-- timeline. `run_id` correlates to backup_reports.run_id the same way
-- multiple backup_reports rows already do for a multi-target schedule (see
-- 20260608110000_backup_reports_run_id.sql); it is not a foreign key for the
-- same reason -- one run_id fans out to several backup_reports rows and,
-- here, to events for both the source and the repository host.
--
-- `agent_id`/`repo_id` identify which of a multi-target schedule's target
-- pairings an event belongs to (the same pairing a backup_reports row for
-- this run_id has) -- run_id alone can't, since every target shares it. The
-- run detail view filters this table's rows down to the pairing it's
-- showing rather than trusting every row for a run_id to be its own.

CREATE TABLE backup_run_events (
    id BIGSERIAL PRIMARY KEY,
    run_id TEXT NOT NULL,
    agent_id BIGINT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    target TEXT NOT NULL
        CHECK (target IN ('source', 'repository')),
    event_type TEXT NOT NULL
        CHECK (event_type IN (
            'reachability_check', 'wake_sent', 'host_online',
            'agent_start_sent', 'agent_connected',
            'agent_stop_sent', 'shutdown_sent', 'host_offline'
        )),
    message TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX backup_run_events_run_id_idx ON backup_run_events (run_id);
CREATE INDEX backup_run_events_target_idx ON backup_run_events (agent_id, repo_id);
