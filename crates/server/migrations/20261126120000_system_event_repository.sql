-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- The repository a system event is about, for the events that concern one
-- (syncs, archive deletion and compaction). A user who is not an admin sees
-- such an event only while they may read that repository.
--
-- No foreign key: an event about a repository that has since been deleted
-- keeps its id and so stays hidden from restricted users, instead of turning
-- into an event about nothing that everyone sees.
ALTER TABLE system_events ADD COLUMN repo_id BIGINT;

CREATE INDEX idx_system_events_repo_id ON system_events (repo_id) WHERE repo_id IS NOT NULL;
