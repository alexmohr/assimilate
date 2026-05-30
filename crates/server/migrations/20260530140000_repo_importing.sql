-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE repos ADD COLUMN importing BOOLEAN NOT NULL DEFAULT FALSE;
