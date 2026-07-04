-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Storage quota shared across all repositories that reside on the same SSH
-- host, for the case where multiple repos share one disk/server.
CREATE TABLE server_quotas (
    ssh_host TEXT PRIMARY KEY,
    warn_bytes BIGINT,
    critical_bytes BIGINT,
    warn_action TEXT NOT NULL DEFAULT 'notify_only'
        CHECK (warn_action IN ('notify_only', 'block_backups', 'disable_schedule')),
    critical_action TEXT NOT NULL DEFAULT 'notify_only'
        CHECK (critical_action IN ('notify_only', 'block_backups', 'disable_schedule')),
    enabled BOOLEAN NOT NULL DEFAULT true,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
