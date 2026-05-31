-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

ALTER TABLE clients ADD COLUMN agent_git_sha TEXT;
ALTER TABLE clients ADD COLUMN agent_build_time TEXT;
