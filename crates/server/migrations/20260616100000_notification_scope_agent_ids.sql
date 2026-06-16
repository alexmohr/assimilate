-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- The client->agent rename (20260610100000_rename_clients_to_agents.sql) missed the
-- "client_ids" key stored inside notification_channels.scope JSONB. Rename it to
-- "agent_ids" so it matches the rest of the agent terminology, preserving existing values.
UPDATE notification_channels
SET scope = (scope - 'client_ids') || jsonb_build_object('agent_ids', scope -> 'client_ids')
WHERE scope ? 'client_ids';
