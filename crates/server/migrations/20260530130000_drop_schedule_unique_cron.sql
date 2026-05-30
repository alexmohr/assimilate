-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE schedules DROP CONSTRAINT IF EXISTS schedules_client_id_repo_id_cron_expression_key;
