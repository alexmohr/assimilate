<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Security: Agent-Repo Assignment Validation

## Objective

Validate that an authenticated agent can only persist backup reports, status updates, and trigger archive indexing for repos it is legitimately assigned to via `schedule_targets`, closing the integrity gap where a compromised agent can forge history for any repo.

## Context

Issue #239: The `BackupStarted`, `BackupCompleted`, `StatusUpdate`, `CheckCompleted`, `VerifyCompleted`, and `CanaryVerified` message handlers in `ws/handler.rs` use `repo_id` directly from the agent message without cross-referencing the `(agent_id, repo_id)` pair against `schedule_targets` → `schedules.repo_id`. Agent identity is correctly authenticated at the WebSocket level, but authority is not scoped to assigned repos. A single compromised agent can forge reports, pollute trends/quotas, and trigger borg/SSH archive indexing on arbitrary repos.

## Steps

1. **Add DB helper** — `check_agent_repo_access(pool, agent_id, repo_id) -> Result<bool, ApiError>` that queries whether any `schedule_targets` row links this agent via a schedule whose `repo_id` matches. Query:

   ```sql
   SELECT EXISTS(
     SELECT 1 FROM schedule_targets st
     JOIN schedules s ON s.id = st.schedule_id
     WHERE st.agent_id = $1 AND s.repo_id = $2
   ) AS "exists!"
   ```

2. **Add a reusable validation function** in `handler.rs` (e.g. `validate_agent_repo`) that calls the DB helper, logs a security warning + system event on failure, and returns a boolean. A single centralized check ensures all message types are covered.

3. **Guard every agent-reported `repo_id` consumer** in `handle_agent_message`:
   - `AgentToServer::BackupStarted` — validate before `insert_backup_started` and `fail_other_started_backups`
   - `AgentToServer::BackupCompleted` — validate before `insert_backup_report` and archive indexing (`ensure_indexed`)
   - `AgentToServer::StatusUpdate` — validate before persisting (currently a no-op; apply guard for future-proofing)
   - `AgentToServer::CheckCompleted` — validate before processing (rejection still logs system event)
   - `AgentToServer::VerifyCompleted` — validate before processing
   - `AgentToServer::CanaryVerified` — validate before processing

4. **On rejection**: log `tracing::warn!(security = true, ...)`, insert a system event via `db::insert_system_event` with type `security_violation`, and skip all persistence/completion-bus/broadcast for that message.

5. **Add integration tests** in `crates/server/tests/db_queries.rs`:
   - Test that an agent assigned to repo A can report for repo A (success)
   - Test that the same agent is **rejected** when reporting for repo B (not assigned)
   - Verify system event is recorded on rejection

## Files

| File | Changes |
|------|---------|
| `crates/server/src/db/mod.rs` | Add `check_agent_repo_access()` function |
| `crates/server/src/ws/handler.rs` | Add `validate_agent_repo()`; guard all 6 message handlers |
| `crates/server/tests/db_queries.rs` | Add integration tests for access check |

## Testing

- **Unit tests**: Not applicable — logic is integration-dependent.
- **Integration tests**: Via `#[sqlx::test]` in `db_queries.rs`, exercising both allowed and denied scenarios with real DB.
- **Manual verification**: Run the existing e2e suite to confirm legitimate backups continue to work unchanged.

## Acceptance Criteria

- [ ] Reports/status for a `repo_id` the agent is not assigned to are rejected and not persisted; a `security_violation` system event is recorded.
- [ ] Archive indexing (`ensure_indexed`) is not triggered for unassigned repos.
- [ ] Legitimate reports (agent assigned to the repo via a schedule) continue to work unchanged.
- [ ] Tests pass: an agent submitting a report for an unrelated `repo_id` is rejected; an assigned agent succeeds.
