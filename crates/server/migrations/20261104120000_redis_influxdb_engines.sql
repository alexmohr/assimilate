-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Agents dump Redis, Valkey and InfluxDB (1 and 2) in pods and containers
-- too. The engine still mirrors the closed enum in `shared::database`.

ALTER TABLE agent_database_sources
    DROP CONSTRAINT agent_database_sources_engine_check,
    ADD CONSTRAINT agent_database_sources_engine_check
        CHECK (engine IN (
            'postgresql', 'mysql', 'mariadb', 'mongodb', 'sqlite',
            'redis', 'valkey', 'influxdb', 'influxdb1'
        ));
