-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Where a channel's notifications link to: where the event happened (the
-- behaviour before this setting existed), the Activity Log, the event's agent,
-- schedule or repository, the dashboard, an address of the operator's own, or
-- nothing. The address is kept only for a `custom` link.
ALTER TABLE notification_channels
    ADD COLUMN link_target TEXT NOT NULL DEFAULT 'automatic'
        CHECK (link_target IN ('automatic', 'activity', 'agent', 'schedule', 'repository',
                               'dashboard', 'custom', 'none')),
    ADD COLUMN link_url TEXT
        CHECK ((link_target = 'custom') = (link_url IS NOT NULL));
