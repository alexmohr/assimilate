# Reset & Re-import: Destroy metadata and full sync from disk

## Objective

Add a "Reset & Re-import" operation that deletes all archive metadata (backup_reports, archives, archive_files, archive_index_jobs, archive_tags) for a repository and performs a fresh full re-import from borg on disk. Both the existing "Full Resync" and the new "Reset & Re-import" must acquire the per-repo `RepoLock` and be available through the scheduler.

## Codebase Context

### The Problem

When archives are imported from disk before a matching agent is registered, placeholder agents are created with `matched = false`. Even after real agents register with the same hostname, the old backup_reports rows still show as unmatched. The existing "Re-scan" button (`POST /api/repos/{repo_id}/rescan`) should re-evaluate these, but the same hostname appears in both "matched" and "unmatched" groups in the UI because both the real agent and the placeholder agent have the same hostname string — the only difference is the `matched` boolean.

The user needs a "nuclear" option to wipe all archive metadata and re-import from scratch so that the fresh import resolves all archives against current agents correctly.

### Repo Lock (`RepoLock`)

**File:** `crates/server/src/lib.rs` (lines 35–59)

```rust
pub struct RepoLock {
    locks: Arc<Mutex<HashMap<i64, Arc<Mutex<()>>>>>,
}
```

- **Per-repo** async mutex (not global). Different repos can be operated concurrently; operations on the same repo are serialized.
- `acquire(repo_id)` returns an `OwnedMutexGuard<()>` that lives for the scope of the operation.
- **Currently acquired by**: scheduled backups (`run_sequential_schedule`), manual "Run Now", archive indexing (`run_indexing`), archive deletion.
- **NOT currently acquired by**: `sync_repo` (API Full Resync) or `run_repo_sync` (scheduler periodic sync). Both only use the DB `importing` flag.

### Scheduler

**File:** `crates/server/src/scheduler.rs`

Four background loops in `run()`:

| Loop | Interval | Function |
|---|---|---|
| `schedule_task` | 30s | `tick()` — run due schedules (backup, check, verify) |
| `retention_task` | 3600s | `run_retention_cleanup` |
| **`sync_task`** | **60s** | **`run_repo_sync`** — periodic archive sync |
| `session_cleanup_task` | 3600s | Delete expired sessions |

`run_repo_sync` (lines 111–315):
- Queries repos with a `sync_schedule` cron expression
- Skips repos currently importing (DB flag check)
- Calculates next run time from `last_synced_at` + cron
- If due: sets `importing=true`, spawns `sync_existing_archives`
- Does NOT currently acquire `RepoLock`

### Existing callers of `sync_existing_archives`

| Location | Context | Has `RepoLock` access? |
|---|---|---|
| `repos.rs:260` | Initial import in `tokio::spawn` | Yes (`state_repo_lock` captured at line 223) |
| `repos.rs:2290` | `sync_repo` API handler | Yes (`state.repo_lock`) |
| `scheduler.rs:200` | `run_repo_sync` | No — needs to be added |
| `archives.rs:531` | Post-delete archive list refresh | Yes (`state.repo_lock`) |
| `main.rs:147` | Startup recovery | No (runs before other ops, less critical) |
| `scheduler.rs:923` | `#[cfg(test)]` test code | Not needed |

### FK chain for deletions

```
repos(id) ON DELETE CASCADE
  ├── backup_reports.repo_id
  ├── archives.repo_id
  │     ├── archive_files.archive_id ON DELETE CASCADE
  │     ├── archive_index_jobs.archive_id ON DELETE CASCADE
  │     └── archive_tags.archive_id ON DELETE CASCADE
  └── archive_paths.repo_id ON DELETE CASCADE
```

`archive_files.path_id → archive_paths(id)` has NO CASCADE — must GC orphaned paths explicitly.

## Implementation Steps

### 1. Add `RepoLock` to `run_repo_sync` (scheduler)

**File:** `crates/server/src/scheduler.rs`

- Add `repo_lock: &RepoLock` parameter to `run_repo_sync` function signature
- Before calling `sync_existing_archives`, acquire the lock:
  ```rust
  let _repo_guard = repo_lock.acquire(repo_id).await;
  ```
- Update the caller in `run()` to pass `&state.repo_lock`

### 2. Acquire `RepoLock` in `sync_repo` (API handler)

**File:** `crates/server/src/api/repos.rs`

- In `sync_repo`, after the `importing` check and before `sync_existing_archives`:
  ```rust
  let _repo_guard = state.repo_lock.acquire(repo_id).await;
  ```
- This ensures API-triggered full resync waits for any concurrent backup on the same repo.

### 3. Acquire `RepoLock` in initial import path

**File:** `crates/server/src/api/repos.rs`

- At line 260, spawned task already has `state_repo_lock` captured (line 223).
- Acquire the lock before `sync_existing_archives`:
  ```rust
  let _repo_guard = state_repo_lock.acquire(repo_id).await;
  ```

### 4. Add DB function: `delete_all_repo_archive_data`

**File:** `crates/server/src/db/mod.rs`

```rust
pub async fn delete_all_repo_archive_data(pool: &PgPool, repo_id: i64) -> Result<u64, ApiError>
```

Transaction body:
1. Collect candidate `path_id`s from `archive_files` for the repo (same pattern as `delete_archive_records_by_names` lines 4412–4423)
2. `DELETE FROM backup_reports WHERE repo_id = $1`
3. `DELETE FROM archives WHERE repo_id = $1` (CASCADES to archive_files, archive_index_jobs, archive_tags)
4. GC orphaned `archive_paths`:
   ```sql
   DELETE FROM archive_paths WHERE repo_id = $1
     AND NOT EXISTS (SELECT 1 FROM archive_files WHERE path_id = archive_paths.id)
     AND NOT EXISTS (SELECT 1 FROM archive_files WHERE parent_path_id = archive_paths.id)
   ```
5. Return count of deleted backup_reports

### 5. Add DB function: `delete_orphaned_placeholder_agents`

**File:** `crates/server/src/db/mod.rs`

Reuse the same query already in `rescan_repo` (line 2228):

```rust
pub async fn delete_orphaned_placeholder_agents(pool: &PgPool) -> Result<u64, ApiError> {
    let result = sqlx::query(
        "DELETE FROM agents WHERE agent_token_hash = 'imported:no-auth'
         AND NOT EXISTS (SELECT 1 FROM backup_reports WHERE agent_id = agents.id)",
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}
```

### 6. Add API handler: `reset_and_sync_repo`

**File:** `crates/server/src/api/repos.rs`

```rust
#[utoipa::path(
    post,
    path = "/api/repos/{repo_id}/reset-and-sync",
    tag = "Repositories",
    operation_id = "resetAndSyncRepo",
    summary = "Delete all archive metadata and re-import from borg",
)]
pub async fn reset_and_sync_repo(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(repo_id): Path<i64>,
) -> Result<Json<SyncResponse>, ApiError>
```

Logic:
1. Fetch repo and verify `!repo.importing` (return 409 if importing)
2. Acquire `RepoLock`: `let _repo_guard = state.repo_lock.acquire(repo_id).await;`
3. Set `importing = true`
4. Call `db::delete_all_repo_archive_data(&state.pool, repo_id)`
5. Call `db::delete_orphaned_placeholder_agents(&state.pool)`
6. Call `sync_existing_archives(...)` — reuse same logic
7. Update `last_synced_at`
8. If `build_index=true` (passed via query), spawn `index_archives_with_progress`
9. Error handling: same pattern as `sync_repo` (log events, clear importing flag)
10. Return `SyncResponse { imported, removed, duration_secs }`

Reuse `SyncQuery` and `SyncResponse` structs already defined near `sync_repo`.

### 7. Register route

**File:** `crates/server/src/main.rs`

After the existing sync route (line 309):
```rust
.route("/api/repos/{repo_id}/reset-and-sync", post(api::repos::reset_and_sync_repo))
```

### 8. Frontend: Add "Reset & Re-import" button + simple confirmation dialog

**File:** `frontend/src/views/RepoDetailView.vue`

**a) State variables:**
```typescript
const resetAndSyncLoading = ref(false)
const showResetAndSyncDialog = ref(false)
```

**b) Handler function** (following `syncRepo` pattern at line 940):
```typescript
async function resetAndSync(): Promise<void> {
  showResetAndSyncDialog.value = false
  resetAndSyncLoading.value = true
  try {
    await apiClient.post(`/repos/${repoId.value}/reset-and-sync?build_index=true`)
    toastSuccess('Archive metadata reset and re-import started.')
    await loadArchives()
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    resetAndSyncLoading.value = false
  }
}
```

**c) Danger Zone entry** (Overview tab, after "Delete Repository" at line 1534):
```html
<div class="danger-body">
  <div class="danger-info">
    <span class="danger-heading">Reset & Re-import</span>
    <span class="danger-desc">
      Delete ALL archive metadata (backup reports, file indexes, tags) and re-import
      from the borg repository on disk. Use this when archives show as unmatched
      despite matching hostnames. The repository data on disk is NOT touched.
    </span>
  </div>
  <button class="btn btn-sm btn-danger" :disabled="resetAndSyncLoading"
    @click="showResetAndSyncDialog = true">
    {{ resetAndSyncLoading ? 'Resetting...' : 'Reset & Re-import' }}
  </button>
</div>
```

**d) Simple confirmation dialog** (following the existing dialog pattern e.g. `showConfirmRelocationDialog`):
- Title: "Reset & Re-import?"
- Body: "This will permanently delete ALL archive metadata for this repository and re-import from borg. This operation cannot be undone. Are you sure?"
- Two buttons: **Cancel** (closes dialog) and **Confirm Reset** (calls `resetAndSync()`)
- No text-input requirement — just a simple confirm/cancel dialog
- The button is already admin-only (`v-if="isAdmin"` on the Danger Zone at line 1456)

### 9. Post-delete sync in `delete_archive` already locked

**File:** `crates/server/src/api/archives.rs` (line 531)

No change needed — `sync_existing_archives` call at line 531 already executes within the `repo_lock` scope acquired at line 428.

## Files to Create or Modify

| File | Action | Purpose |
|---|---|---|
| `crates/server/src/scheduler.rs` | Modify | Add `repo_lock` param to `run_repo_sync`, acquire lock before sync |
| `crates/server/src/api/repos.rs` | Modify | Add `RepoLock` acquisition in `sync_repo` and initial import path; add `reset_and_sync_repo` handler |
| `crates/server/src/db/mod.rs` | Modify | Add `delete_all_repo_archive_data` and `delete_orphaned_placeholder_agents` functions |
| `crates/server/src/main.rs` | Modify | Register new route `/api/repos/{repo_id}/reset-and-sync`; pass `repo_lock` to `run_repo_sync` |
| `frontend/src/views/RepoDetailView.vue` | Modify | Add "Reset & Re-import" button + simple confirmation dialog in Danger Zone |

## Testing Approach

### Backend tests

1. **`delete_all_repo_archive_data` test** (in `crates/server/tests/db_queries.rs`):
   - Create a repo with backup_reports, archives, archive_files, archive_tags
   - Call `delete_all_repo_archive_data`
   - Verify backup_reports == 0, archives == 0, archive_files == 0, archive_paths GC'd
   - Verify repo still exists

2. **`delete_orphaned_placeholder_agents` test** (in `db_queries.rs`):
   - Placeholder agent with no backup_reports → verify deleted
   - Placeholder agent with backup_reports → verify NOT deleted

3. **`reset_and_sync_repo` integration test** (in `crates/server/tests/integration.rs`):
   - Set up repo with known borg archives
   - Call reset+sync endpoint
   - Verify archives re-imported with correct `matched` status

### Frontend tests

- Playwright e2e test: navigate to repo overview, click "Reset & Re-import", verify confirmation dialog, cancel, then confirm.

## Acceptance Criteria

1. `run_repo_sync` acquires `RepoLock` before syncing.
2. `sync_repo` API handler acquires `RepoLock` before syncing.
3. Initial import path acquires `RepoLock` before syncing.
4. `POST /api/repos/{repo_id}/reset-and-sync` deletes ALL backup_reports, archives, archive_files, archive_index_jobs, archive_tags, and orphaned archive_paths for the repo.
5. Orphaned placeholder agents are cleaned up after the reset.
6. Returns 409 Conflict if the repo is already importing.
7. Triggers a full re-import via `sync_existing_archives`, re-resolving all archives against current agents.
8. Existing "Full Resync" is completely unaffected.
9. Frontend shows "Reset & Re-import" in the Danger Zone (admin-only) with a simple Cancel/Confirm dialog.
10. All tests pass: `cargo test --workspace`, `npm run build`, `npm run lint`.
