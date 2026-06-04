-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE schedules ADD COLUMN keep_hourly INTEGER NOT NULL DEFAULT 24;
