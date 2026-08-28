-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Lets a schedule wake the host an agent runs on (Wake-on-LAN) when it is
-- not already reachable, and optionally start the agent process itself over
-- SSH on hosts where it is not a persistent background service. Both are
-- undone after the backup only if that same run is what turned them on --
-- see crates/server/src/power.rs.

ALTER TABLE agents
    ADD COLUMN wake_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN wake_mac_address TEXT,
    ADD COLUMN wake_broadcast_address TEXT,
    ADD COLUMN wake_timeout_seconds INTEGER NOT NULL DEFAULT 180
        CHECK (wake_timeout_seconds > 0),
    ADD COLUMN shutdown_after_backup BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN start_agent_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN stop_agent_after_backup BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN ssh_host TEXT,
    ADD COLUMN ssh_port INTEGER NOT NULL DEFAULT 22,
    ADD COLUMN agent_service_name TEXT NOT NULL DEFAULT 'assimilate-agent';

-- A MAC address is meaningless without wake enabled, and vice versa a
-- wake-enabled host needs one to send a magic packet to.
ALTER TABLE agents
    ADD CONSTRAINT agents_wake_requires_mac
    CHECK (NOT wake_enabled OR wake_mac_address IS NOT NULL);

ALTER TABLE agents
    ADD CONSTRAINT agents_wake_mac_format
    CHECK (wake_mac_address IS NULL
        OR wake_mac_address ~* '^([0-9A-F]{2}:){5}[0-9A-F]{2}$');

-- Shutting down only makes sense for a host this feature might have woken.
ALTER TABLE agents
    ADD CONSTRAINT agents_shutdown_requires_wake
    CHECK (NOT shutdown_after_backup OR wake_enabled);

-- Stopping the agent process only makes sense if this run might have
-- started it.
ALTER TABLE agents
    ADD CONSTRAINT agents_stop_agent_requires_start
    CHECK (NOT stop_agent_after_backup OR start_agent_enabled);

-- Starting the agent over SSH needs somewhere to connect to.
ALTER TABLE agents
    ADD CONSTRAINT agents_start_agent_requires_ssh_host
    CHECK (NOT start_agent_enabled OR ssh_host IS NOT NULL);
