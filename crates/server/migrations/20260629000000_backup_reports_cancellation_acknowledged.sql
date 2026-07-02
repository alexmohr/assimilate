-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE backup_reports
    ADD COLUMN cancellation_acknowledged BOOLEAN NOT NULL DEFAULT false;
