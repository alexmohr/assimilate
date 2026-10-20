-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Agents dump MySQL, MariaDB and MongoDB databases too, next to PostgreSQL,
-- in pods, in containers and installed on the host. The engine still mirrors
-- the closed enum in `shared::database`.

ALTER TABLE agent_database_sources
    DROP CONSTRAINT agent_database_sources_engine_check,
    ADD CONSTRAINT agent_database_sources_engine_check
        CHECK (engine IN ('postgresql', 'mysql', 'mariadb', 'mongodb'));

-- MySQL or MariaDB installed on the host. Its clients run as an OS user that
-- signs in on its own (socket authentication or its ~/.my.cnf); no password
-- is stored here.
ALTER TABLE agents
    ADD COLUMN database_host_mysql_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN database_host_mysql_engine TEXT NOT NULL DEFAULT 'mysql'
        CHECK (database_host_mysql_engine IN ('mysql', 'mariadb')),
    -- NULL is root.
    ADD COLUMN database_host_mysql_os_user TEXT,
    -- The server's Unix socket. NULL is the client's default.
    ADD COLUMN database_host_mysql_socket TEXT
        CHECK (database_host_mysql_socket LIKE '/%'),
    -- A TCP port on 127.0.0.1. NULL connects over the socket.
    ADD COLUMN database_host_mysql_port INTEGER
        CHECK (database_host_mysql_port BETWEEN 1 AND 65535),
    ADD COLUMN database_host_mysql_bin_dir TEXT
        CHECK (database_host_mysql_bin_dir LIKE '/%');

-- MongoDB installed on the host. A server with access control on is named by
-- a config file of the database tools on the host, holding its uri; its path
-- is stored here, never its content.
ALTER TABLE agents
    ADD COLUMN database_host_mongodb_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    -- NULL is root.
    ADD COLUMN database_host_mongodb_os_user TEXT,
    -- A port on 127.0.0.1. NULL is 27017.
    ADD COLUMN database_host_mongodb_port INTEGER
        CHECK (database_host_mongodb_port BETWEEN 1 AND 65535),
    ADD COLUMN database_host_mongodb_config_file TEXT
        CHECK (database_host_mongodb_config_file LIKE '/%'),
    ADD COLUMN database_host_mongodb_bin_dir TEXT
        CHECK (database_host_mongodb_bin_dir LIKE '/%');
