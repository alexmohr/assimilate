-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

CREATE TABLE per_agent_commands (
    schedule_id BIGINT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    agent_id    BIGINT NOT NULL REFERENCES agents(id)    ON DELETE CASCADE,
    pre_backup_commands  TEXT NOT NULL DEFAULT '[]',
    post_backup_commands TEXT NOT NULL DEFAULT '[]',
    CONSTRAINT per_agent_commands_pk PRIMARY KEY (schedule_id, agent_id)
);
