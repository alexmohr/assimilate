<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)

See the NOTICE file(s) distributed with this work for additional
information regarding copyright ownership.

This program and the accompanying materials are made available under the
terms of the Apache License Version 2.0 which is available at
https://www.apache.org/licenses/LICENSE-2.0
-->

# Plan: Remove `--format` from `borg list` in full resync to fix hang with many archives

## Objective

Fix the full-resync "Listing archives…" hang by removing the redundant `--format '{hostname}{end}'` flag from the `borg list --json` command in `run_borg_list_with_retry`.

## Codebase context

- **`crates/server/src/api/repos.rs`** — `run_borg_list_with_retry()` (line 1404) runs:

  ```bash
  borg list --json --format '{hostname}{end}' --lock-wait 60 <repo>
  ```

  This command is used by `sync_archives()` during full resync to enumerate all archives in the repository. The `--format` flag was added to ensure `hostname` and `end` fields are present in the JSON output (line 1413-1414).

- **`crates/server/src/api/search.rs`** — `list_archives_sorted()` (line 285) runs the same command **without `--format`**:

  ```bash
  borg list --json --lock-wait 60 <repo>
  ```

  This works fine — no hang, instant with 142+ archives.

- **`build_import_reports()`** (repos.rs:1589) consumes the JSON and accesses fields `name`, `hostname`, `start`, `end` from each archive entry. These fields are already included in borg's default `--json` output in all widely-used borg versions (1.1+, 1.2+, 2.x).

## Root cause

The `--format '{hostname}{end}'` flag triggers a different code path in borg that can hang internally for repos with many archives. This is a borg bug where the format-string evaluation path has different lock/IO behavior or a serialization issue that doesn't occur with plain `--json`.

The same command **without** `--format` works instantly (proven by `list_archives_sorted`).

## Implementation steps

1. In `crates/server/src/api/repos.rs`, modify `run_borg_list_with_retry()` (around line 1416):
   - Remove `"--format"` and `"{hostname}{end}"` from the args array
   - Update the doc comment above the args to explain that `hostname` and `end` are included by default in borg's JSON output

2. Update the doc comment on `build_import_reports()` (line 1582-1588) to remove the reference to `--format '{hostname}{end}'`.

## Files to create or modify

| File | Change |
|------|--------|
| `crates/server/src/api/repos.rs` | Remove `--format`/`{hostname}{end}` from `run_borg_list_with_retry` args; update comments (lines ~1413-1420 and ~1582-1588) |

## Testing approach

1. **Unit test**: No new tests needed — the existing tests already mock `borg list` output. The JSON format without `--format` is already tested in the search path (`list_archives_sorted`).

2. **Integration test**: Verify that `borg list --json` (without `--format`) still includes `hostname` and `end` fields. The existing integration test at `crates/server/tests/integration.rs:2219` already tests the `--format` behavior — update it to verify the default JSON has these fields instead.

3. **Manual verification**: Run a full resync on a repo with many archives and confirm:
   - "Listing archives…" phase completes within seconds
   - Archive names resolve with correct hostnames
   - End timestamps are populated correctly

## Acceptance criteria

1. `borg list --json --lock-wait 60 <repo>` (without `--format`) completes immediately for repos with 142+ archives
2. Full resync "Listing archives…" phase no longer hangs
3. Archive hostnames and end timestamps are still correctly imported (verified by `build_import_reports`)
4. All existing tests pass
5. The `--format` doc comments are updated to reflect that it was removed and why
