-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- A schedule can now write to more than one repository, so that a single read
-- of the source hosts produces several independent copies.
--
-- `schedules.repo_id` stays, denormalised, as the schedule's primary target -
-- the lowest `execution_order` row below. Health summaries, quota accounting,
-- reports and the manual-run endpoints all key off it, and keeping it in sync
-- means multi-target is additive rather than a rewrite of everything that
-- reads a schedule's repository.

CREATE TABLE schedule_repos (
    id BIGSERIAL PRIMARY KEY,
    schedule_id BIGINT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    repo_id BIGINT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    execution_order INTEGER NOT NULL DEFAULT 0,
    -- A required target's failure is the schedule's failure: it counts towards
    -- the failure streak and, with on_failure = 'stop', ends the run for that
    -- host. A best-effort target only warns.
    required BOOLEAN NOT NULL DEFAULT TRUE,
    UNIQUE (schedule_id, repo_id)
);

CREATE INDEX idx_schedule_repos_schedule ON schedule_repos (schedule_id, execution_order);
CREATE INDEX idx_schedule_repos_repo ON schedule_repos (repo_id);

INSERT INTO schedule_repos (schedule_id, repo_id, execution_order, required)
SELECT id, repo_id, 0, TRUE
FROM schedules
WHERE repo_id IS NOT NULL;
