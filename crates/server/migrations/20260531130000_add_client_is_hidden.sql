-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Add is_hidden flag to clients for hiding imported clients from all views
ALTER TABLE clients ADD COLUMN is_hidden BOOLEAN NOT NULL DEFAULT false;
CREATE INDEX idx_clients_is_hidden ON clients (is_hidden) WHERE is_hidden = true;
