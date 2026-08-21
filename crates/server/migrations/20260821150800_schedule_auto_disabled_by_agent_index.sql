-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- reenable_system_disabled_schedules_for_agent runs this WHERE clause on every agent
-- WebSocket reconnect, not just ones with auto-disabled schedules. A partial index
-- matching the query exactly keeps that lookup cheap regardless of schedule table size
-- or reconnect churn.
CREATE INDEX idx_schedules_auto_disabled_by_agent_id ON schedules (auto_disabled_by_agent_id)
    WHERE auto_disabled_agent_unreachable = true;
