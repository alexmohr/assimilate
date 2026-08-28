-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- How many consecutive missed backups (agent unreachable, target/config error
-- at trigger time) a schedule tolerates before it is marked failed and
-- auto-disabled, replacing the previous hardcoded threshold of 3. Below this
-- count a miss only shows as a warning.
ALTER TABLE schedules
    ADD COLUMN missed_backup_threshold INTEGER NOT NULL DEFAULT 3
    CHECK (missed_backup_threshold > 0);
