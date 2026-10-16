-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Lets an agent find databases in Docker or Podman containers and on the
-- host itself, next to its Kubernetes cluster. Each runtime is turned on per
-- host; a host that already dumped databases keeps searching its cluster.

ALTER TABLE agents
    ADD COLUMN database_kubernetes_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN database_docker_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    -- The engine's API socket. NULL uses $DOCKER_HOST, or
    -- /var/run/docker.sock.
    ADD COLUMN database_docker_socket TEXT
        CHECK (database_docker_socket LIKE '/%'),
    ADD COLUMN database_host_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    -- The OS user host dumps run as. NULL is `postgres`. Peer authentication
    -- lets it in as the role of the same name; no password is stored here.
    ADD COLUMN database_host_os_user TEXT,
    -- Directory of the server's Unix socket. NULL is the client's default.
    ADD COLUMN database_host_socket_dir TEXT
        CHECK (database_host_socket_dir LIKE '/%'),
    -- The server's port. NULL is the client's default.
    ADD COLUMN database_host_port INTEGER
        CHECK (database_host_port BETWEEN 1 AND 65535),
    -- Directory holding pg_dump and psql. NULL finds them on the agent's PATH.
    ADD COLUMN database_host_bin_dir TEXT
        CHECK (database_host_bin_dir LIKE '/%');

UPDATE agents SET database_kubernetes_enabled = database_backup_enabled;

-- A source is now told apart by the runtime it lives in as well: a container
-- and a pod may carry the same name. A namespace is empty where a runtime
-- has none.
ALTER TABLE agent_database_sources
    ADD COLUMN runtime TEXT NOT NULL DEFAULT 'kubernetes'
        CHECK (runtime IN ('kubernetes', 'docker', 'host')),
    DROP CONSTRAINT agent_database_sources_agent_id_namespace_name_key,
    ADD CONSTRAINT agent_database_sources_source_key UNIQUE (agent_id, runtime, namespace, name),
    DROP CONSTRAINT agent_database_sources_kind_check,
    ADD CONSTRAINT agent_database_sources_kind_check
        CHECK (kind IN ('cnpg_cluster', 'annotated', 'container', 'host'));
