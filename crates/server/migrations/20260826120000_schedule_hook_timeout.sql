-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Pre/post-backup hook commands previously ran under a hard-coded 60s
-- timeout on the agent (crates/agent/src/backup.rs). Make it configurable
-- per schedule so a hook that legitimately needs longer (e.g. a live disk
-- snapshot commit) isn't force-killed.

ALTER TABLE schedules
    ADD COLUMN hook_timeout_seconds INTEGER NOT NULL DEFAULT 60
    CHECK (hook_timeout_seconds > 0);
