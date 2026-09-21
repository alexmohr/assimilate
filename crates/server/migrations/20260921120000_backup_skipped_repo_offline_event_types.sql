-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- shared::types::SystemEventType and notifications::EventType both gained
-- BackupSkippedRepoOffline, recorded/dispatched when the scheduler skips a
-- target because the host holding its repository does not answer SSH. The
-- sibling backup_skipped_agent_offline covers the agent being away; this one
-- covers a connected agent whose backup destination is not there, which until
-- now surfaced only as a plain backup failure once borg tried to reach it -
-- extend the closed sets system_events.event_type and
-- notification_rules.event_type are locked to so db::insert_system_event and a
-- notification rule for this event can actually be created instead of
-- violating the constraints.
ALTER TABLE system_events DROP CONSTRAINT system_events_event_type_check;

ALTER TABLE system_events
    ADD CONSTRAINT system_events_event_type_check
    CHECK (event_type IN (
        'repo_sync', 'repo_sync_slow', 'repo_sync_failed', 'repo_sync_cancelled',
        'archive_delete_failed', 'archive_compact_failed', 'account_locked',
        'auth_failed', 'security_violation', 'schedule_auto_disabled',
        'schedule_reenabled', 'schedule_catch_up', 'backup_skipped_agent_offline',
        'backup_skipped_repo_offline'
    )) NOT VALID;

ALTER TABLE notification_rules DROP CONSTRAINT notification_rules_event_type_check;

ALTER TABLE notification_rules
    ADD CONSTRAINT notification_rules_event_type_check
    CHECK (event_type IN (
        'backup_success', 'backup_warning', 'backup_failed',
        'check_success', 'check_failed',
        'agent_connected', 'agent_disconnected',
        'schedule_auto_disabled', 'backup_skipped_agent_offline',
        'backup_skipped_repo_offline'
    )) NOT VALID;
