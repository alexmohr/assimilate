-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE backup_reports
    ADD COLUMN IF NOT EXISTS schedule_id BIGINT REFERENCES schedules(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_backup_reports_schedule_id ON backup_reports (schedule_id);
