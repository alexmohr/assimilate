-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- shared::types::SystemEventType gained ArchiveCompactFailed (the compact
-- that automatically follows an archive delete can fail independently of
-- the delete itself); extend the closed set the previous migration locked
-- system_events.event_type to so db::insert_system_event can actually
-- record it instead of silently violating the constraint.
ALTER TABLE system_events DROP CONSTRAINT system_events_event_type_check;

ALTER TABLE system_events
    ADD CONSTRAINT system_events_event_type_check
    CHECK (event_type IN (
        'repo_sync', 'repo_sync_slow', 'repo_sync_failed', 'repo_sync_cancelled',
        'archive_delete_failed', 'archive_compact_failed', 'account_locked',
        'auth_failed', 'security_violation'
    )) NOT VALID;
