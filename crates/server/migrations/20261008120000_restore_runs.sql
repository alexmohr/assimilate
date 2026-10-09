-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- One row per restore of archive files onto an agent's filesystem. A restore
-- used to be a request the API waited 30 seconds on, while `borg extract` on
-- the agent can run for half an hour; now the API records the restore here
-- and returns at once, and the agent's RestoreCompleted (or OperationFailed)
-- settles the row whenever it arrives.
--
-- `id` is the request id sent to the agent. `pending` means the restore has
-- not been handed to the agent yet (it is offline, and the restore goes out
-- when it reconnects); `running` means it has. `agent_instance` is the
-- agent process the restore was handed to (from its Hello), so a reconnect
-- from a restarted agent, which lost the restore and the answer it would
-- have sent, can fail it instead of leaving it running forever.

CREATE TABLE restore_runs (
    id UUID PRIMARY KEY,
    agent_id BIGINT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    archive_name TEXT NOT NULL,
    paths TEXT[] NOT NULL,
    target_path TEXT NOT NULL,
    status TEXT NOT NULL
        CHECK (status IN ('pending', 'running', 'success', 'failed', 'cancelled')),
    files_restored BIGINT,
    error_message TEXT,
    requested_by TEXT NOT NULL,
    agent_instance TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ
);

CREATE INDEX restore_runs_created_at_idx ON restore_runs (created_at DESC);
-- The reconnect catch-up looks up an agent's pending and running restores.
CREATE INDEX restore_runs_agent_status_idx ON restore_runs (agent_id, status);
