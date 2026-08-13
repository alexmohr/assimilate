-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Serves list_archives()'s `DISTINCT ON (archive_name) ... WHERE repo_id = $1 AND archive_name
-- IS NOT NULL ORDER BY archive_name, started_at DESC, id DESC` query. Without this, the only
-- index touching this table's repo_id column is the plain single-column
-- idx_backup_reports_repo_id, so the DISTINCT ON has to sort every matching row from scratch --
-- on a repo with years of retained archives, that's thousands of rows on every page load.
CREATE INDEX idx_backup_reports_repo_archive_started_id
    ON backup_reports(repo_id, archive_name, started_at DESC, id DESC)
    WHERE archive_name IS NOT NULL;
