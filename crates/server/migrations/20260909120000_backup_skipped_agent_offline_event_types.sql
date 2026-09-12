-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- shared::types::SystemEventType and notifications::EventType both gained
-- BackupSkippedAgentOffline, recorded/dispatched when the scheduler skips a
-- required target because its agent is offline (not yet enough consecutive
-- misses to auto-disable the schedule) - extend the closed sets
-- system_events.event_type and notification_rules.event_type are locked to
-- so db::insert_system_event and a notification rule for this event can
-- actually be created instead of violating the constraints.
ALTER TABLE system_events DROP CONSTRAINT system_events_event_type_check;

ALTER TABLE system_events
    ADD CONSTRAINT system_events_event_type_check
    CHECK (event_type IN (
        'repo_sync', 'repo_sync_slow', 'repo_sync_failed', 'repo_sync_cancelled',
        'archive_delete_failed', 'archive_compact_failed', 'account_locked',
        'auth_failed', 'security_violation', 'schedule_auto_disabled',
        'schedule_reenabled', 'schedule_catch_up', 'backup_skipped_agent_offline'
    )) NOT VALID;

ALTER TABLE notification_rules DROP CONSTRAINT notification_rules_event_type_check;

ALTER TABLE notification_rules
    ADD CONSTRAINT notification_rules_event_type_check
    CHECK (event_type IN (
        'backup_success', 'backup_warning', 'backup_failed',
        'check_success', 'check_failed',
        'agent_connected', 'agent_disconnected',
        'schedule_auto_disabled', 'backup_skipped_agent_offline'
    )) NOT VALID;
