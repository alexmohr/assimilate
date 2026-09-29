-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- notifications::EventType gained two variants, each split out of an event
-- that used to cover it so a channel can follow one without the other:
--
-- * backup_file_changed: a backup whose only warnings were files changing
--   while borg read them. Used to be a plain backup_warning.
-- * backup_catch_up_abandoned: a run missed by a host marked as not always
--   online was dropped because the host did not come back within its give-up
--   window. Used to be a plain backup_failed.
--
-- Only notification_rules changes; neither is a system event of its own.
ALTER TABLE notification_rules DROP CONSTRAINT notification_rules_event_type_check;

ALTER TABLE notification_rules
    ADD CONSTRAINT notification_rules_event_type_check
    CHECK (event_type IN (
        'backup_success', 'backup_warning', 'backup_failed',
        'check_success', 'check_failed',
        'agent_connected', 'agent_disconnected',
        'schedule_auto_disabled', 'backup_skipped_agent_offline',
        'backup_skipped_repo_offline', 'backup_file_changed',
        'backup_catch_up_abandoned'
    )) NOT VALID;

-- A channel that heard about these through the event they were split out of
-- keeps hearing about them: every existing rule for the old event gets a twin
-- for the new one, with the same scope and enabled state. Anyone who wants
-- them apart turns the new toggle off.
INSERT INTO notification_rules (channel_id, event_type, repo_id, agent_id, schedule_id, enabled)
SELECT channel_id, 'backup_file_changed', repo_id, agent_id, schedule_id, enabled
FROM notification_rules
WHERE event_type = 'backup_warning';

INSERT INTO notification_rules (channel_id, event_type, repo_id, agent_id, schedule_id, enabled)
SELECT channel_id, 'backup_catch_up_abandoned', repo_id, agent_id, schedule_id, enabled
FROM notification_rules
WHERE event_type = 'backup_failed';
