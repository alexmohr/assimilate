-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- System events that report a problem (a failed periodic repo sync, an
-- auto-disabled schedule, ...) can be acknowledged the same way a warning or
-- failed backup report can, so a reviewed problem stops counting against the
-- unreviewed feed. Informational events are never acknowledged, so they simply
-- keep the default.
ALTER TABLE system_events
    ADD COLUMN acknowledged BOOLEAN NOT NULL DEFAULT false;

CREATE INDEX idx_system_events_acknowledged_created_at
    ON system_events (acknowledged, created_at DESC);
