-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Multiple hosts can report the same OS hostname while living under
-- different DNS domains. The agent itself has no reliable way to learn its
-- own domain, so it is an optional value an admin sets manually to
-- disambiguate such hosts.
ALTER TABLE agents ADD COLUMN domain TEXT;

-- Original constraint predates the clients->agents rename, which never
-- renamed it (see 20260610100000_rename_clients_to_agents.sql).
ALTER TABLE agents DROP CONSTRAINT clients_hostname_key;

-- COALESCE keeps today's behavior for the common case (no domain set):
-- hostname alone stays unique. Once a domain is set, (hostname, domain)
-- pairs are what must be unique.
CREATE UNIQUE INDEX agents_hostname_domain_idx
    ON agents (hostname, COALESCE(domain, ''));
