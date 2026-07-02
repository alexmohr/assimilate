-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Add file_change_patterns_raw column to schedules table
ALTER TABLE schedules ADD COLUMN file_change_patterns_raw TEXT NOT NULL DEFAULT '';

-- Create per_agent_file_change_patterns table
CREATE TABLE IF NOT EXISTS per_agent_file_change_patterns (
    schedule_id BIGINT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    agent_id BIGINT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    raw_text TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (schedule_id, agent_id)
);
