-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Tracks how many consecutive times a schedule failed to reach its agent(s)
-- so the scheduler can back off (advance next_run_at instead of retrying
-- every tick) and eventually stop trying, rather than hammering an offline
-- agent forever. auto_disabled_agent_unreachable marks a schedule the
-- scheduler disabled for this reason specifically, so reconnect handling
-- only ever re-enables schedules it disabled itself - never ones a human or
-- quota enforcement disabled for an unrelated reason.
ALTER TABLE schedules
    ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN auto_disabled_agent_unreachable BOOLEAN NOT NULL DEFAULT FALSE;
