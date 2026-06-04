-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE schedules ALTER COLUMN repo_id DROP NOT NULL;
ALTER TABLE schedules DROP CONSTRAINT IF EXISTS schedules_repo_id_fkey;
ALTER TABLE schedules ADD CONSTRAINT schedules_repo_id_fkey
    FOREIGN KEY (repo_id) REFERENCES repos(id) ON DELETE SET NULL;
