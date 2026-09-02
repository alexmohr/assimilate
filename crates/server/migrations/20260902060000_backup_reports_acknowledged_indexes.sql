-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- `acknowledged` became a filter dimension on backup_reports rather than just a
-- display flag: the dashboard summary now skips reviewed runs, and the Activity
-- Log asks on every load what is still outstanding. Following the precedent in
-- 20260813150000_backup_reports_perf_indexes.sql, each of those gets an index
-- rather than leaning on an existing one that does not cover the new predicate.

-- Serves get_dashboard_summary()'s last_failure_*/last_warning_* CTEs
-- (status = 'failed'/'warning' AND acknowledged = false ORDER BY finished_at
-- DESC LIMIT 1), on every dashboard load. The existing
-- (status, finished_at DESC) index finds the newest matching runs but then has
-- to skip every acknowledged one, which degrades exactly as a long-lived
-- instance accumulates reviewed failures newer than the oldest unreviewed one.
-- Partial, because the acknowledged rows are precisely the ones these queries
-- never want, and leaving them out keeps the index small and the writes cheap.
CREATE INDEX idx_backup_reports_unacknowledged_status_finished_at
    ON backup_reports (status, finished_at DESC)
    WHERE acknowledged = false;

-- Serves the repository-scoped outstanding count and bulk acknowledge
-- (repo_id = ANY(..) AND acknowledged = false AND status IN (..)), which the
-- Acknowledge all button's visibility probe runs on every Activity Log load.
CREATE INDEX idx_backup_reports_unacknowledged_repo_status
    ON backup_reports (repo_id, status)
    WHERE acknowledged = false;
