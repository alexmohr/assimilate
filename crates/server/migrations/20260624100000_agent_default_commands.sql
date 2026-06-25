-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE agents
    ADD COLUMN default_pre_backup_commands TEXT NOT NULL DEFAULT '[]',
    ADD COLUMN default_post_backup_commands TEXT NOT NULL DEFAULT '[]';
