-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
--
-- See the NOTICE file(s) distributed with this work for additional
-- information regarding copyright ownership.
--
-- This program and the accompanying materials are made available under the
-- terms of the Apache License Version 2.0 which is available at
-- https://www.apache.org/licenses/LICENSE-2.0

-- Add is_hidden flag to clients for hiding imported clients from all views
ALTER TABLE clients ADD COLUMN is_hidden BOOLEAN NOT NULL DEFAULT false;
CREATE INDEX idx_clients_is_hidden ON clients (is_hidden) WHERE is_hidden = true;
