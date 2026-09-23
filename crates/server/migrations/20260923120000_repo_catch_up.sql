-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Whether a host is expected to be reachable is a fact about the host, not
-- about any schedule that happens to use it: two schedules writing to the same
-- NAS cannot sensibly disagree on whether that NAS sleeps. So the switch, and
-- how long to wait for the host to come back, live on the agent and on the
-- repository - and schedules.catch_up_missed_runs, which used to decide this
-- once per schedule for every host it touched, goes away.
--
-- intermittent is "host is not always online". Off - the default, and right
-- for a server that should always be there - an unreachable host is a failed
-- backup like any other error. On, it is expected: the run is reported as
-- skipped and caught up once the host is back.
--
-- catch_up_give_up_minutes bounds that wait, measured from the missed
-- occurrence. Zero waits for as long as it takes, the same way keep_yearly = 0
-- means "keep none" rather than needing a nullable column.
ALTER TABLE agents
    ADD COLUMN intermittent BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN catch_up_give_up_minutes INTEGER NOT NULL DEFAULT 0
    CHECK (catch_up_give_up_minutes >= 0);

-- A repository has one setting an agent does not: an agent announces its own
-- return by reconnecting its websocket, so its catch-up is event-driven, while
-- a repository has no connection to announce anything on and has to be asked
-- over SSH. catch_up_recheck_minutes is how often.
ALTER TABLE repos
    ADD COLUMN intermittent BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN catch_up_recheck_minutes INTEGER NOT NULL DEFAULT 15
    CHECK (catch_up_recheck_minutes > 0),
    ADD COLUMN catch_up_give_up_minutes INTEGER NOT NULL DEFAULT 0
    CHECK (catch_up_give_up_minutes >= 0);

-- Dropped rather than carried over. Nothing is migrated onto the hosts: a
-- schedule-wide opt-in says nothing reliable about which of its hosts is the
-- one that sleeps, so whoever relied on it marks those hosts instead.
-- catch_up_min_lead_minutes stays on schedules - it is measured against that
-- schedule's own next run, and a nightly and a weekly schedule writing to the
-- same host need different values.
ALTER TABLE schedules
    DROP COLUMN catch_up_missed_runs;

-- Which occurrence a repository missed, per (schedule, repository), matching
-- schedule_targets.catch_up_pending_for's per (schedule, agent). No agent
-- dimension: a repository that was down for an occurrence was down for every
-- agent writing to it in that window. A single timestamp rather than a
-- counter, so a later miss overwrites it and however many occurrences pass,
-- exactly one catch-up run follows.
--
-- catch_up_run_id / catch_up_run_for remember the catch-up run a marker was
-- handed to and the occurrence it stood for. The marker itself is cleared
-- when the run is dispatched; if that very run fails against the repository
-- again, the new marker takes the original occurrence back instead of the
-- retry's own start, so retries never push the give-up window forward.
ALTER TABLE schedule_repos
    ADD COLUMN catch_up_pending_for TIMESTAMPTZ,
    ADD COLUMN catch_up_last_probe_at TIMESTAMPTZ,
    ADD COLUMN catch_up_run_id TEXT,
    ADD COLUMN catch_up_run_for TIMESTAMPTZ;

-- The poller scans this whole (small) partial index every pass; a
-- repository's own Power pane looks itself up in it.
CREATE INDEX idx_schedule_repos_catch_up_pending
    ON schedule_repos (repo_id)
    WHERE catch_up_pending_for IS NOT NULL;

-- shared::types::SystemEventType gained two variants:
--
-- * backup_failed_agent_offline: an agent that is *not* marked intermittent
--   was not connected when its backup came due. No backup report exists for a
--   run that never started, so this is the Activity Log's only record of it -
--   the red counterpart of backup_skipped_agent_offline, which is now reserved
--   for agents that are expected to be away.
-- * schedule_catch_up_abandoned: a pending catch-up outlived its host's
--   give-up window and was dropped.
--
-- Only the system_events set changes. Both go out as a plain backup_failed
-- notification, so a rule that already alerts on failed backups covers them
-- without anyone adding a rule for an event they have never seen.
ALTER TABLE system_events DROP CONSTRAINT system_events_event_type_check;

ALTER TABLE system_events
    ADD CONSTRAINT system_events_event_type_check
    CHECK (event_type IN (
        'repo_sync', 'repo_sync_slow', 'repo_sync_failed', 'repo_sync_cancelled',
        'archive_delete_failed', 'archive_compact_failed', 'account_locked',
        'auth_failed', 'security_violation', 'schedule_auto_disabled',
        'schedule_reenabled', 'schedule_catch_up', 'backup_skipped_agent_offline',
        'backup_skipped_repo_offline', 'schedule_catch_up_abandoned',
        'backup_failed_agent_offline'
    )) NOT VALID;
