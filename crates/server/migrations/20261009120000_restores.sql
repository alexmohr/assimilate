-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- A restore of archive files onto an agent, recorded so it can outlive the
-- request that started it. `borg extract` can take far longer than any HTTP
-- request should stay open, and a restore to an offline agent waits until the
-- agent reconnects, so the API returns this row at once and the UI follows it.
--
-- `request_id` is the id the server and agent use for the restore over the
-- WebSocket (RestoreFiles / RestoreStarted / RestoreCompleted). It is unique,
-- so an agent answer always names one row.
--
-- `status` mirrors shared::types::RestoreStatus:
--
-- * queued: the agent was offline; the restore is sent once it reconnects.
-- * dispatched: sent to the agent, which has not started extracting yet.
-- * running: the agent started `borg extract`.
-- * succeeded / failed / cancelled: final.
CREATE TABLE restores (
    id BIGSERIAL PRIMARY KEY,
    request_id TEXT NOT NULL UNIQUE,
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    agent_id BIGINT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    archive_name TEXT NOT NULL,
    paths TEXT[] NOT NULL,
    target_path TEXT NOT NULL,
    requested_by TEXT NOT NULL,
    status TEXT NOT NULL
        CHECK (status IN (
            'queued', 'dispatched', 'running', 'succeeded', 'failed', 'cancelled'
        )),
    files_restored BIGINT,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ
);

-- An agent's reconnect looks up the restores it still owes an answer for.
CREATE INDEX restores_unfinished_agent_idx ON restores (agent_id)
    WHERE status IN ('queued', 'dispatched', 'running');
-- Retention prunes finished restores by finished_at.
CREATE INDEX restores_finished_at_idx ON restores (finished_at);

-- shared::types::SystemEventType gained restore_completed and restore_failed,
-- the Activity Log's record of how a restore ended.
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
        'restore_completed', 'restore_failed'
    )) NOT VALID;
