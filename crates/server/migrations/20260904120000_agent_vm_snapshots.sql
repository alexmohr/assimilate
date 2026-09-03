-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Lets an agent stage the libvirt/QEMU domains of its host into a directory
-- before a backup runs, so the virtual machines end up in the archive as
-- ordinary files. The settings live on the agent because the staging
-- directory belongs to the host: two schedules backing up the same host would
-- otherwise each claim it with different limits. A schedule only opts in.

ALTER TABLE agents
    ADD COLUMN vm_snapshot_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN vm_snapshot_dir TEXT NOT NULL DEFAULT '/home/virt/backups',
    ADD COLUMN vm_snapshot_full_interval INTEGER NOT NULL DEFAULT 7
        CHECK (vm_snapshot_full_interval > 0),
    ADD COLUMN vm_snapshot_timeout_seconds INTEGER NOT NULL DEFAULT 1800
        CHECK (vm_snapshot_timeout_seconds > 0),
    -- Bytes one domain may occupy below the staging directory. 0 is unlimited.
    ADD COLUMN vm_snapshot_default_limit_bytes BIGINT NOT NULL DEFAULT 0
        CHECK (vm_snapshot_default_limit_bytes >= 0);

-- Staging into a relative path would resolve against the agent's working
-- directory, which is not something the operator can see or reason about.
ALTER TABLE agents
    ADD CONSTRAINT agents_vm_snapshot_dir_absolute
    CHECK (vm_snapshot_dir LIKE '/%');

-- The domains the agent last reported, plus the per-domain settings the
-- operator made. Rows survive a rescan so an override outlives a host reboot,
-- and are deleted with their agent.
CREATE TABLE agent_vms (
    id BIGSERIAL PRIMARY KEY,
    agent_id BIGINT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    -- Operator settings.
    included BOOLEAN NOT NULL DEFAULT TRUE,
    -- NULL inherits the agent's default limit.
    limit_bytes BIGINT CHECK (limit_bytes IS NULL OR limit_bytes >= 0),
    -- Last scan.
    state TEXT NOT NULL DEFAULT 'unknown',
    mode TEXT NOT NULL DEFAULT 'unknown',
    disk_count INTEGER NOT NULL DEFAULT 0 CHECK (disk_count >= 0),
    disk_bytes BIGINT NOT NULL DEFAULT 0 CHECK (disk_bytes >= 0),
    -- Last run.
    staged_bytes BIGINT NOT NULL DEFAULT 0 CHECK (staged_bytes >= 0),
    chain_length INTEGER NOT NULL DEFAULT 0 CHECK (chain_length >= 0),
    last_error TEXT,
    last_scanned_at TIMESTAMPTZ,
    last_staged_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (agent_id, name)
);

CREATE INDEX idx_agent_vms_agent ON agent_vms(agent_id);

-- A schedule stages the host's virtual machines only when it opts in, so a
-- second schedule on the same host can back up plain files without paying for
-- a snapshot run.
ALTER TABLE schedules
    ADD COLUMN vm_snapshot_enabled BOOLEAN NOT NULL DEFAULT FALSE;
