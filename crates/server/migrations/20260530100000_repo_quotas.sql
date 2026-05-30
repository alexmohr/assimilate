-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

CREATE TABLE repo_quotas (
    repo_id BIGINT PRIMARY KEY REFERENCES repos(id) ON DELETE CASCADE,
    warn_bytes BIGINT,
    critical_bytes BIGINT,
    enabled BOOLEAN NOT NULL DEFAULT true,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
