-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE backup_reports ADD COLUMN run_id TEXT;
CREATE INDEX idx_backup_reports_run_id ON backup_reports(run_id) WHERE run_id IS NOT NULL;
