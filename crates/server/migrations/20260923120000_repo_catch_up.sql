-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Catching up an occurrence missed because the *repository* host was away, the
-- mirror of what schedule_targets already carries for the agent case.
--
-- The two halves are not symmetric and cannot be. An agent announces its own
-- return by reconnecting its websocket, so its catch-up is event-driven and
-- immediate. A repository has no connection to announce anything on, so it has
-- to be asked over SSH: catch_up_repo_recheck_minutes is how often, and
-- schedule_repos.catch_up_last_probe_at is when it was last asked.
ALTER TABLE schedules
    ADD COLUMN catch_up_repo_recheck_minutes INTEGER NOT NULL DEFAULT 15
    CHECK (catch_up_repo_recheck_minutes > 0),
    -- How long a pending catch-up may stay pending before it is abandoned and
    -- the run reported as failed. Zero is "wait for as long as it takes",
    -- which is the behaviour catch-up shipped with, so no existing schedule
    -- changes meaning on upgrade - the same way keep_yearly = 0 means "keep
    -- none" rather than needing a nullable column. Measured from the missed
    -- occurrence, not from the last probe, so the budget is the wall-clock
    -- wait an operator actually asked for.
    ADD COLUMN catch_up_give_up_minutes INTEGER NOT NULL DEFAULT 0
    CHECK (catch_up_give_up_minutes >= 0);

-- Per (schedule, repository), matching schedule_targets.catch_up_pending_for's
-- per (schedule, agent). The agent dimension is deliberately absent: a
-- repository that was down for an occurrence was down for every agent writing
-- to it in that window, so recording it per agent would store the same fact
-- once per target and produce one catch-up run each.
--
-- A single timestamp rather than a counter, for the same reason as the agent
-- column: a later miss overwrites it, which is what collapses however many
-- missed occurrences into exactly one catch-up run.
ALTER TABLE schedule_repos
    ADD COLUMN catch_up_pending_for TIMESTAMPTZ,
    ADD COLUMN catch_up_last_probe_at TIMESTAMPTZ;

-- The poller scans this whole (small) partial index every pass; "check now"
-- looks up one repository in it.
CREATE INDEX idx_schedule_repos_catch_up_pending
    ON schedule_repos (repo_id)
    WHERE catch_up_pending_for IS NOT NULL;

-- shared::types::SystemEventType gained ScheduleCatchUpAbandoned, recorded when
-- a pending catch-up outlives catch_up_give_up_minutes and is dropped. Only the
-- system_events set changes: the notification that goes out alongside it is a
-- plain backup_failed, so that a rule which already alerts on failed backups
-- covers this without anyone adding a rule for an event they have never seen.
ALTER TABLE system_events DROP CONSTRAINT system_events_event_type_check;

ALTER TABLE system_events
    ADD CONSTRAINT system_events_event_type_check
    CHECK (event_type IN (
        'repo_sync', 'repo_sync_slow', 'repo_sync_failed', 'repo_sync_cancelled',
        'archive_delete_failed', 'archive_compact_failed', 'account_locked',
        'auth_failed', 'security_violation', 'schedule_auto_disabled',
        'schedule_reenabled', 'schedule_catch_up', 'backup_skipped_agent_offline',
        'backup_skipped_repo_offline', 'schedule_catch_up_abandoned'
    )) NOT VALID;
