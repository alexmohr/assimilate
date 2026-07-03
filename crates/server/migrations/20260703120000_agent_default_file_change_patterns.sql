-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Add agent-level default file change patterns, mirroring default_exclude_patterns.
-- Applies to every schedule targeting this agent, as a fallback for warnings not
-- matched by any schedule-level (or per-agent-schedule override) pattern.
ALTER TABLE agents ADD COLUMN default_file_change_patterns_raw TEXT NOT NULL DEFAULT '';
