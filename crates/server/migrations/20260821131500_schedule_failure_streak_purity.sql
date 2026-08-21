-- SPDX-License-Identifier: Apache-2.0
-- SPDX-FileCopyrightText: 2026 Alexander Mohr

-- Tracks whether every failure in the schedule's current consecutive-failure streak
-- has been a connectivity (agent-unreachable) failure, so a streak containing even one
-- local/data failure (e.g. a transient config-assembly error) never gets marked
-- auto_disabled_agent_unreachable just because the specific failure that happened to
-- cross the threshold was itself a connectivity failure. Reset to true together with
-- consecutive_failures everywhere that column resets.
ALTER TABLE schedules
    ADD COLUMN failure_streak_pure_connectivity BOOLEAN NOT NULL DEFAULT TRUE;
