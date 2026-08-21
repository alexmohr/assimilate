-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Tracks which specific target agent's failures most recently drove a schedule's
-- consecutive-failure count, so reconnect handling only re-enables a schedule when
-- *that* agent reconnects - not merely any of the schedule's targets. Without this, a
-- multi-target schedule with one permanently-unreachable target and one merely-flaky
-- target would have its auto-disable bookkeeping reset every time the flaky target's
-- routine reconnects happened, never actually reaching MAX_CONSECUTIVE_FAILURES for
-- the target that's really broken.
ALTER TABLE schedules
    ADD COLUMN auto_disabled_by_agent_id BIGINT REFERENCES agents(id) ON DELETE SET NULL;
