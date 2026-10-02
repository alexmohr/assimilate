-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- When an archive's content index was last read by the archive browser.
--
-- The `archive_index_retention_days` setting evicts the content index of an
-- archive nobody has browsed for that many days. An index built long ago but
-- browsed yesterday is still in use, so `finished_at` alone is the wrong basis:
-- eviction keys on the later of `finished_at` and `last_accessed_at`.
--
-- NULL means the index has not been browsed since it was built (or since this
-- column was added), in which case `finished_at` stands in for it.
ALTER TABLE archive_index_jobs ADD COLUMN last_accessed_at TIMESTAMPTZ;
