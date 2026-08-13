-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Several columns model a fixed set of states as free TEXT with no CHECK
-- constraint, unlike other enum-like columns in this schema (e.g.
-- notification_channels.channel_type, notification_rules.event_type,
-- notification_deliveries.status, tags.scope, agents.visibility,
-- repos.visibility). Bring them in line so the DB backstops the Rust-side
-- enums instead of accepting any string.

-- shared::types::BackupStatus ('success', 'warning', 'failed') plus the
-- transient in-flight states the DB layer itself writes while a backup is
-- running (see insert_backup_pending/insert_backup_started/
-- cancel_backup_report in db/mod.rs): 'pending' while queued, 'started'
-- once the agent confirms, 'cancelled' if aborted before completion.
ALTER TABLE backup_reports
    ADD CONSTRAINT backup_reports_status_check
    CHECK (status IN ('pending', 'started', 'cancelled', 'success', 'warning', 'failed'));

-- Closed set of event types the server itself ever inserts (see call sites of
-- db::insert_system_event); not user- or agent-supplied. NOT VALID: older
-- deployments may carry now-retired event_type values (e.g. a historical
-- 'agent_connected' event) that predate this constraint, and get_system_events
-- is deliberately tolerant of such rows (skips rather than fails the whole
-- query) -- validating existing data here would break the migration on any
-- database that still has one.
ALTER TABLE system_events
    ADD CONSTRAINT system_events_event_type_check
    CHECK (event_type IN (
        'repo_sync', 'repo_sync_slow', 'repo_sync_failed', 'repo_sync_cancelled',
        'archive_delete_failed', 'account_locked', 'auth_failed', 'security_violation'
    )) NOT VALID;

-- shared::types::ScheduleType
ALTER TABLE schedules
    ADD CONSTRAINT schedules_schedule_type_check
    CHECK (schedule_type IN ('backup', 'check', 'verify'));

-- crate::archive_index::IndexStatus
ALTER TABLE archive_index_jobs
    ADD CONSTRAINT archive_index_jobs_status_check
    CHECK (status IN ('pending', 'indexing', 'done', 'failed'));

-- shared::protocol::RepoOpKind. The `last_op_kind` column moved from `repos`
-- to `repo_last_op.kind` in 20260703225141_repo_satellite_tables.sql.
ALTER TABLE repo_last_op
    ADD CONSTRAINT repo_last_op_kind_check
    CHECK (kind IN (
        'agent_backup', 'agent_check', 'agent_verify',
        'server_sync', 'break_lock', 'delete_archive'
    ));

-- schedules.execution_mode already had a CHECK, but 20260608100000 migrated
-- every row to 'sequential' and changed the default, leaving 'parallel'
-- permitted but dead. shared::types::ExecutionMode now only has one variant
-- (it still *parses* "parallel" as an input alias for backward compatibility,
-- but never stores it) -- tighten the constraint to match what can actually
-- be persisted.
ALTER TABLE schedules DROP CONSTRAINT schedules_execution_mode_check;
ALTER TABLE schedules
    ADD CONSTRAINT schedules_execution_mode_check
    CHECK (execution_mode IN ('sequential'));
