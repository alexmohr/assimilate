-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- db::list_auto_disabled_schedule_ids_for_agent runs on every agent WebSocket
-- reconnect (via ws/handler.rs::reenable_system_disabled_schedules_on_reconnect), not
-- just ones with auto-disabled schedules. It filters schedules by
-- auto_disabled_agent_unreachable = true, joined through schedule_targets rather than
-- an equality lookup on auto_disabled_by_agent_id - this partial index still keeps
-- that filter cheap by letting Postgres scan only the currently-auto-disabled rows,
-- regardless of schedule table size or reconnect churn. auto_disabled_by_agent_id is
-- the indexed column mainly so equality lookups on it (e.g.
-- db::reenable_system_disabled_schedules_for_agent, kept as a lower-level primitive
-- and exercised directly by tests) stay cheap too.
CREATE INDEX idx_schedules_auto_disabled_by_agent_id ON schedules (auto_disabled_by_agent_id)
    WHERE auto_disabled_agent_unreachable = true;
