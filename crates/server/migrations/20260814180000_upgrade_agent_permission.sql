-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE roles ADD COLUMN can_upgrade_agent BOOLEAN NOT NULL DEFAULT false;
UPDATE roles SET can_upgrade_agent = true WHERE name = 'admin';
