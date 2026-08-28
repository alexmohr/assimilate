-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- notifications::EventType gained ScheduleAutoDisabled, dispatched when the
-- scheduler auto-disables a schedule after it reaches its
-- missed_backup_threshold - extend the closed set notification_rules.event_type
-- is locked to so a rule can actually be created for it.
ALTER TABLE notification_rules DROP CONSTRAINT notification_rules_event_type_check;

ALTER TABLE notification_rules
    ADD CONSTRAINT notification_rules_event_type_check
    CHECK (event_type IN (
        'backup_success', 'backup_warning', 'backup_failed',
        'check_success', 'check_failed',
        'agent_connected', 'agent_disconnected',
        'schedule_auto_disabled'
    )) NOT VALID;
