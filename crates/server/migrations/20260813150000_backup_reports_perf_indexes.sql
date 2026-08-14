-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Performance indexes for backup_reports, the largest and fastest-growing table on a
-- long-lived instance with many retained backups. Without these, the dashboard summary and
-- health-check queries -- which look up "the most recent report matching X" -- have to sort
-- the entire filtered row set from scratch on every request.

-- Serves get_dashboard_summary()'s per-status "latest report" lookups
-- (status = 'success'/'failed'/'warning' ... ORDER BY finished_at DESC LIMIT 1) and any other
-- reporting query that filters by status and wants the most recent match.
CREATE INDEX idx_backup_reports_status_finished_at ON backup_reports(status, finished_at DESC);

-- Serves the status-agnostic "most recent report with a schedule_id" lookup in
-- get_dashboard_summary() (last_backup_schedule_id), which orders by finished_at without a
-- status filter.
CREATE INDEX idx_backup_reports_finished_at ON backup_reports(finished_at DESC);

-- Serves the per-(schedule, agent) "most recent report" lookup used once per row by
-- dashboard::targets() and get_health_summary() on every dashboard/health load.
CREATE INDEX idx_backup_reports_schedule_agent_started_at
    ON backup_reports(schedule_id, agent_id, started_at DESC);
