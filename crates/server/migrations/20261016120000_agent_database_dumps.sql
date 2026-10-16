-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Lets an agent dump the databases running in its Kubernetes cluster into a
-- backup's archive. Like virtual-machine staging, the settings live on the
-- agent because the cluster and the dump directory belong to the host; a
-- schedule only opts in.

ALTER TABLE agents
    ADD COLUMN database_backup_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    -- Where the agent creates the dump pipes and the manifest. The archive
    -- stores the dumps under this path.
    ADD COLUMN database_dump_dir TEXT NOT NULL DEFAULT '/var/lib/assimilate/database-dumps',
    -- Kubeconfig file on the agent host. NULL uses the in-cluster service
    -- account, or $KUBECONFIG and ~/.kube/config. A path, never the file's
    -- content: the credentials stay on the host.
    ADD COLUMN database_kubeconfig_path TEXT,
    -- Which direction agent_database_sources.included is read in, mirroring
    -- `shared::database::DatabaseSelectionMode`. The default keeps a database
    -- deployed after the last scan backed up rather than silently missed.
    ADD COLUMN database_selection TEXT NOT NULL DEFAULT 'all'
        CHECK (database_selection IN ('all', 'selected'));

ALTER TABLE agents
    ADD CONSTRAINT agents_database_dump_dir_absolute
    CHECK (database_dump_dir LIKE '/%');

-- The database workloads the agent last reported, plus the operator's choice
-- for each. A row is kept across rescans while it carries a choice, and is
-- deleted with its agent.
CREATE TABLE agent_database_sources (
    id BIGSERIAL PRIMARY KEY,
    agent_id BIGINT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    namespace TEXT NOT NULL,
    name TEXT NOT NULL,
    -- Operator setting. NULL is a source nobody decided about; the agent's
    -- database_selection then says whether it is dumped.
    included BOOLEAN,
    -- Last scan. NULL until a scan has seen the source. Both mirror closed
    -- enums in `shared::database`.
    kind TEXT CHECK (kind IN ('cnpg_cluster', 'annotated')),
    engine TEXT CHECK (engine IN ('postgresql')),
    pod TEXT,
    container TEXT,
    databases TEXT[] NOT NULL DEFAULT '{}',
    last_error TEXT,
    last_scanned_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (agent_id, namespace, name)
);

CREATE INDEX idx_agent_database_sources_agent ON agent_database_sources(agent_id);

ALTER TABLE schedules
    ADD COLUMN database_backup_enabled BOOLEAN NOT NULL DEFAULT FALSE;
