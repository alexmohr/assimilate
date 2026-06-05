-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE schedules ADD COLUMN ignore_changed_files BOOLEAN NOT NULL DEFAULT FALSE;
