-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

CREATE TABLE server_quotas (
    ssh_host TEXT PRIMARY KEY,
    warn_bytes BIGINT,
    critical_bytes BIGINT,
    warn_action TEXT NOT NULL DEFAULT 'notify_only',
    critical_action TEXT NOT NULL DEFAULT 'notify_only',
    enabled BOOLEAN NOT NULL DEFAULT true,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
