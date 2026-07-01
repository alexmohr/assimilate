-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Add archive_name column to backup_reports
ALTER TABLE backup_reports ADD COLUMN archive_name TEXT;
