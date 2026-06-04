-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Store the repository's total unique compressed size (cache.stats.unique_csize from borg)
-- at the time of each backup. This is the actual on-disk usage of the repository,
-- as opposed to backup_reports.deduplicated_size which is the per-archive delta.
ALTER TABLE backup_reports ADD COLUMN repo_unique_csize BIGINT NOT NULL DEFAULT 0;
