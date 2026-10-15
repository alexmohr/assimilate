-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- One row per hold a running backup target keeps on a power-managed host:
-- the agent's own host, or the repository host it writes to (directly, for
-- the post-backup sync, or because a dependency it needs is on that
-- machine). The server counts these holds in memory and the last one to end
-- shuts the machine down if the session woke it; this table mirrors them so a
-- restart mid-run does not forget a machine it woke.
--
-- `run_id`, `agent_id` and `repo_id` say whose hold it is; a dependency's
-- hold on a repository host has no `repo_id`. `woke` and `started_agent` are
-- the host session's flags as of this hold, kept on every hold of the host,
-- so whichever survives a restart still carries what the session did.
--
-- On startup every row is a hold whose run lost the server that was
-- watching it. It is released once that run's report is settled (or there
-- is none), or left for good after a day without one.

CREATE TABLE power_holds (
    id BIGSERIAL PRIMARY KEY,
    run_id TEXT NOT NULL,
    agent_id BIGINT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    repo_id BIGINT REFERENCES repos(id) ON DELETE CASCADE,
    host_kind TEXT NOT NULL CHECK (host_kind IN ('agent', 'repo_host')),
    host_id BIGINT NOT NULL,
    woke BOOLEAN NOT NULL DEFAULT false,
    started_agent BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_power_holds_host ON power_holds (host_kind, host_id);
CREATE INDEX idx_power_holds_run_id ON power_holds (run_id);

-- shared::types::SystemEventType gained power_release_abandoned: a hold that
-- survived a restart whose run never reported back within a day was dropped
-- without shutting its machine down.
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
        'backup_waiting_for_host', 'machine_migrated', 'power_release_abandoned'
    )) NOT VALID;
