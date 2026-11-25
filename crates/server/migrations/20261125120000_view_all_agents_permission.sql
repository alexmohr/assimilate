-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Lets a role see every agent, including a new one no schedule backs up yet.
-- Without it a user sees only the agents that back up to a repository they
-- may read. The built-in operator manages agents, so it holds it too.
ALTER TABLE roles ADD COLUMN can_view_all_agents BOOLEAN NOT NULL DEFAULT false;
UPDATE roles SET can_view_all_agents = true WHERE name IN ('admin', 'operator');
