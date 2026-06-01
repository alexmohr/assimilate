-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE repos ADD COLUMN import_progress INTEGER NOT NULL DEFAULT 0;
ALTER TABLE repos ADD COLUMN import_total INTEGER NOT NULL DEFAULT 0;
