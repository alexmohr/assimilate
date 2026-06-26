-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE sessions ADD COLUMN remember_me BOOLEAN NOT NULL DEFAULT FALSE;
