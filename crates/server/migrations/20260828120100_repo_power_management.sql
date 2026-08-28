-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Lets a schedule wake the host a repository lives on (Wake-on-LAN) when it
-- is not already reachable over SSH, and shut it back down after the backup
-- only if that same run is what woke it. Unlike an agent host, a repository
-- host has no Assimilate process to start or stop -- it is purely an SSH
-- destination borg writes to, so there is no start/stop-agent counterpart
-- here (see crates/server/src/power.rs). Connection details reuse the
-- repository's existing ssh_host/ssh_port/ssh_user.

ALTER TABLE repos
    ADD COLUMN wake_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN wake_mac_address TEXT,
    ADD COLUMN wake_broadcast_address TEXT,
    ADD COLUMN wake_timeout_seconds INTEGER NOT NULL DEFAULT 180
        CHECK (wake_timeout_seconds > 0),
    ADD COLUMN shutdown_after_backup BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE repos
    ADD CONSTRAINT repos_wake_requires_mac
    CHECK (NOT wake_enabled OR wake_mac_address IS NOT NULL);

ALTER TABLE repos
    ADD CONSTRAINT repos_wake_mac_format
    CHECK (wake_mac_address IS NULL
        OR wake_mac_address ~* '^([0-9A-F]{2}:){5}[0-9A-F]{2}$');

ALTER TABLE repos
    ADD CONSTRAINT repos_shutdown_requires_wake
    CHECK (NOT shutdown_after_backup OR wake_enabled);
