-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE repo_quotas
    ADD COLUMN warn_action TEXT NOT NULL DEFAULT 'notify_only',
    ADD COLUMN critical_action TEXT NOT NULL DEFAULT 'notify_only';
