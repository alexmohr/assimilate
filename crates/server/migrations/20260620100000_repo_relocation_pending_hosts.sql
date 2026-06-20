-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

CREATE TABLE repo_relocation_pending_hosts (
    repo_id  BIGINT NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    hostname TEXT   NOT NULL,
    PRIMARY KEY (repo_id, hostname)
);
