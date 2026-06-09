-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

CREATE TABLE dismissed_dashboard_findings (
    user_id    INT  NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    finding_id TEXT NOT NULL,
    dismissed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, finding_id)
);
