-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

CREATE TABLE client_hostname_patterns (
    id BIGSERIAL PRIMARY KEY,
    client_id BIGINT NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
    pattern TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_client_hostname_patterns_pattern ON client_hostname_patterns(pattern);

ALTER TABLE backup_reports ADD COLUMN matched BOOLEAN NOT NULL DEFAULT true;
