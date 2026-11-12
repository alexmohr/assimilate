-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Append-only repositories. The repository host enforces it, not borg's
-- repository config: the key the agents use is restricted in the host's
-- `authorized_keys` to `borg serve --append-only`, and the server keeps a
-- second, full-access key that it alone uses. sshd runs one forced command per
-- key per login, so the setting belongs to a login (host and SSH user), and
-- every repository that logs in that way is append-only together.

-- A row means the login is append-only.
CREATE TABLE append_only_logins (
    repo_host_id BIGINT NOT NULL REFERENCES repo_hosts(id) ON DELETE CASCADE,
    ssh_user TEXT NOT NULL CHECK (ssh_user <> ''),
    installed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- The last time each key was tried against the host.
    checked_at TIMESTAMPTZ,
    backup_key_ok BOOLEAN,
    maintenance_key_ok BOOLEAN,
    restriction_in_place BOOLEAN,
    check_error TEXT,
    PRIMARY KEY (repo_host_id, ssh_user)
);

-- The server prunes and compacts an append-only repository itself, on this
-- schedule, because the agents' key cannot delete.
CREATE TABLE repo_maintenance (
    repo_id BIGINT PRIMARY KEY REFERENCES repos(id) ON DELETE CASCADE,
    cron_expression TEXT NOT NULL DEFAULT '30 4 * * 0',
    check_before_prune BOOLEAN NOT NULL DEFAULT TRUE,
    next_run_at TIMESTAMPTZ,
    last_run_at TIMESTAMPTZ,
    -- Set while maintenance waits for an admin to look at archives that
    -- vanished from the repository without Assimilate deleting them.
    stopped_at TIMESTAMPTZ
);

CREATE TABLE repo_maintenance_runs (
    id BIGSERIAL PRIMARY KEY,
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at TIMESTAMPTZ,
    outcome TEXT NOT NULL CHECK (outcome IN ('ok', 'stopped', 'check_failed', 'failed')),
    archives_removed BIGINT NOT NULL DEFAULT 0 CHECK (archives_removed >= 0),
    bytes_freed BIGINT NOT NULL DEFAULT 0,
    detail TEXT
);

CREATE INDEX idx_repo_maintenance_runs_repo ON repo_maintenance_runs (repo_id, started_at DESC);

-- An archive that is gone from `borg list` although nobody asked for it to be
-- deleted. On an append-only repository the sync records it here instead of
-- dropping its records, so maintenance can see the deletion before its first
-- full-access write makes it permanent.
CREATE TABLE repo_vanished_archives (
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    archive_name TEXT NOT NULL,
    vanished_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (repo_id, archive_name)
);

-- An archive a user deleted in the UI. On an append-only repository the
-- deletion waits here for the next maintenance run.
CREATE TABLE repo_pending_deletions (
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    archive_name TEXT NOT NULL,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    requested_by TEXT,
    PRIMARY KEY (repo_id, archive_name)
);

-- Maintenance is a repository operation like the others, and compact_repo was
-- missing from the list.
ALTER TABLE repo_last_op DROP CONSTRAINT repo_last_op_kind_check;
ALTER TABLE repo_last_op
    ADD CONSTRAINT repo_last_op_kind_check
    CHECK (kind IN (
        'agent_backup', 'agent_check', 'agent_verify',
        'server_sync', 'break_lock', 'delete_archive',
        'compact_repo', 'maintenance'
    ));
