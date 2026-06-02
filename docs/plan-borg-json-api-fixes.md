<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

# Plan: Align Borg JSON API Usage

Fixes for gaps between the [BorgBackup JSON API](https://borgbackup.readthedocs.io/en/stable/internals/frontends.html)
and how the agent crate currently invokes and parses borg output.

## 1. Fix `parse_dry_run_output` — `file_status` has no `size` field

- **File:** `crates/agent/src/executor.rs` — `parse_dry_run_output`
- **Problem:** The function reads a `size` field from each JSON line, but
  `file_status` events (`{"type": "file_status", "status": "U", "path": "..."}`)
  do not contain `size`. The accumulated `total_size` is therefore always 0.
- **Fix options:**
  - (a) Parse `archive_progress` events instead (which have `original_size`)
    and use the final `archive_progress` with `finished: true` (or the last
    emitted one) to get the totals.
  - (b) Remove `total_size` from the dry-run result if per-file sizes are
    not available in dry-run mode.
- **Tests:** Add a unit test with realistic `file_status` + `archive_progress`
  JSON lines to verify correct parsing.

## 2. Add `--log-json` to `borg create` and parse warnings by `msgid`

- **File:** `crates/agent/src/backup.rs` — `borg_create_args`, `run_borg_create`,
  `parse_warnings`
- **Problem:** Regular `borg create` does not pass `--log-json`, so stderr is
  plain text. Warning detection uses fragile string matching
  (`stderr.contains("file changed while we")`, `line.starts_with("WARNING")`).
- **Fix:**
  - Add `--log-json` to `borg_create_args`.
  - Replace the text-based exit-code-1 check with structured JSON parsing of
    stderr lines, matching on `type == "log_message"` and known `msgid` values
    (`FileChangedWarning`, `BackupWarning`, `BackupError`, etc.).
  - Rewrite `parse_warnings` to extract warnings from JSON log objects.
- **Tests:** Unit tests with sample JSON log lines for warnings and errors.

## 3. Set `BORG_DISPLAY_PASSPHRASE=no` on `borg init`

- **File:** `crates/agent/src/executor.rs` — `run_init_repo_task`
- **Problem:** The docs state that `borg init` prompts to display the
  passphrase. Without `BORG_DISPLAY_PASSPHRASE=no` the process may hang
  waiting for interactive input when running headless.
- **Fix:** Add `.env("BORG_DISPLAY_PASSPHRASE", "no")` to the command builder.
- **Tests:** Existing init tests should still pass; verify no prompt is emitted.

## 4. Set UTF-8 locale environment variables

- **Files:** `crates/agent/src/backup.rs` — `borg_env`,
  `crates/agent/src/executor.rs` — `build_borg_env`
- **Problem:** Borg currently depends on locale being UTF-8
  (see [borg #2273](https://github.com/borgbackup/borg/issues/2273)).
  If the agent runs in a minimal container with `LANG=C`, JSON output
  may contain non-UTF-8 bytes, causing `serde_json` parsing to silently fail.
- **Fix:** In both `borg_env` and `build_borg_env`, add:
  - `LANG=en_US.UTF-8`
  - `LC_CTYPE=en_US.UTF-8`
- **Tests:** Verify env vars are present in the built env vector.

## 5. Add `--log-json` to remaining borg commands

- **Files:** `crates/agent/src/backup.rs` — `run_borg_prune`, `run_borg_compact`,
  `run_borg_check`; `crates/agent/src/executor.rs` — `run_restore_task`
- **Problem:** These commands parse stderr as plain text or ignore it entirely.
  Structured errors/warnings are lost.
- **Fix:** Add `--log-json` to each command and parse stderr JSON lines for
  `log_message` entries with `levelname` of `WARNING` or `ERROR`. Surface
  these in the result or log them with tracing.
- **Priority:** Lower than items 1–4. Can be done incrementally per command.

## Order of work

1. Item 3 (one-line fix, prevents potential hang)
2. Item 4 (small fix, prevents silent parse failures)
3. Item 1 (fixes incorrect data being reported)
4. Item 2 (most impactful change, touches parsing logic)
5. Item 5 (incremental improvement)
