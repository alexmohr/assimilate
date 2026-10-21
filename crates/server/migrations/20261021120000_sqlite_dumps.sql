-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Agents dump SQLite database files too, in pods, in containers and on the
-- host. The engine still mirrors the closed enum in `shared::database`.

ALTER TABLE agent_database_sources
    DROP CONSTRAINT agent_database_sources_engine_check,
    ADD CONSTRAINT agent_database_sources_engine_check
        CHECK (engine IN ('postgresql', 'mysql', 'mariadb', 'mongodb', 'sqlite'));

-- SQLite database files on the host. There is no server and no account:
-- sqlite3 reads each file directly, as an OS user allowed to.
ALTER TABLE agents
    ADD COLUMN database_host_sqlite_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    -- NULL is root.
    ADD COLUMN database_host_sqlite_os_user TEXT,
    -- The absolute paths of the database files.
    ADD COLUMN database_host_sqlite_files TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN database_host_sqlite_bin_dir TEXT
        CHECK (database_host_sqlite_bin_dir LIKE '/%');
