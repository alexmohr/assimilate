-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Agents dump MySQL, MariaDB and MongoDB databases too, next to PostgreSQL.
-- The engine still mirrors the closed enum in `shared::database`.

ALTER TABLE agent_database_sources
    DROP CONSTRAINT agent_database_sources_engine_check,
    ADD CONSTRAINT agent_database_sources_engine_check
        CHECK (engine IN ('postgresql', 'mysql', 'mariadb', 'mongodb'));
