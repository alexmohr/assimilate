-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- A schedule could already carve out custom exclude patterns; this adds the
-- inverse - custom include patterns, which rescue paths from a broader
-- exclude (global, agent-default, or the schedule's own) instead of adding
-- to it. Mirrors exclude_patterns_raw/per_agent_excludes exactly.
ALTER TABLE schedules ADD COLUMN include_patterns_raw TEXT NOT NULL DEFAULT '';

CREATE TABLE per_agent_includes (
    id BIGSERIAL PRIMARY KEY,
    schedule_id BIGINT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    agent_id BIGINT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    raw_text TEXT NOT NULL DEFAULT '',
    UNIQUE (schedule_id, agent_id)
);

CREATE INDEX idx_per_agent_includes_schedule_id ON per_agent_includes(schedule_id);
CREATE INDEX idx_per_agent_includes_agent_id ON per_agent_includes(agent_id);
