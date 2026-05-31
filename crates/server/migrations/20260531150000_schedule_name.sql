-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE schedules ADD COLUMN name TEXT NOT NULL DEFAULT '';
