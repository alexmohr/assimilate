-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- An API token is either a full token, acting as the user who created it on
-- every API endpoint, or a monitoring token, which can only read the
-- Prometheus `/metrics` endpoint and is refused everywhere else. Every token
-- issued before this migration keeps the full scope it always had.
ALTER TABLE api_tokens
    ADD COLUMN scope TEXT NOT NULL DEFAULT 'full'
        CHECK (scope IN ('full', 'monitoring'));
