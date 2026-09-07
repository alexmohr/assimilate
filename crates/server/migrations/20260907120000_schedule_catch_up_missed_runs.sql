-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Opt-in catch-up for occurrences missed while a target host was unreachable.
-- catch_up_min_lead_minutes is the floor that keeps a catch-up from colliding
-- with the regular run it would otherwise land on top of: below that much time
-- until next_run_at, the pending miss is dropped instead of run. It is > 0 so a
-- schedule the reconnect handler just re-enabled (which sets next_run_at = now)
-- can never both catch up and immediately run on the next scheduler tick.
ALTER TABLE schedules
    ADD COLUMN catch_up_missed_runs BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN catch_up_min_lead_minutes INTEGER NOT NULL DEFAULT 120
    CHECK (catch_up_min_lead_minutes > 0);

-- Which occurrence this target missed, per target rather than per schedule so
-- one host coming back does not re-run the backup for targets that never missed
-- anything. Deliberately a single timestamp and not a counter: a later miss
-- overwrites it, which is what makes missed runs collapse into one catch-up
-- however long the host was away.
ALTER TABLE schedule_targets
    ADD COLUMN catch_up_pending_for TIMESTAMPTZ;

-- The reconnect handler's only lookup: pending targets for one agent.
CREATE INDEX idx_schedule_targets_catch_up_pending
    ON schedule_targets (agent_id)
    WHERE catch_up_pending_for IS NOT NULL;

-- shared::types::SystemEventType gained ScheduleCatchUp, recorded when a
-- reconnecting agent triggers a catch-up run - extend the closed set the
-- earlier migration locked system_events.event_type to so db::insert_system_event
-- can record it instead of violating the constraint.
ALTER TABLE system_events DROP CONSTRAINT system_events_event_type_check;

ALTER TABLE system_events
    ADD CONSTRAINT system_events_event_type_check
    CHECK (event_type IN (
        'repo_sync', 'repo_sync_slow', 'repo_sync_failed', 'repo_sync_cancelled',
        'archive_delete_failed', 'archive_compact_failed', 'account_locked',
        'auth_failed', 'security_violation', 'schedule_auto_disabled',
        'schedule_reenabled', 'schedule_catch_up'
    )) NOT VALID;
