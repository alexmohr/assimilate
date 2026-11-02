# mock-borg

Small test double for `borg` used by integration tests.

## Environment variables

- `MOCK_BORG_LOG=/path/to/log` — logs every invocation, one line per call.
- `MOCK_BORG_FAIL=1` — simulates a borg connection failure and exits `2`.
- `MOCK_BORG_SIMULATE_WARNING=1` — simulates a `file changed` warning and exits `1`, for `create` and `extract`.
- `MOCK_BORG_SIMULATE_UNEXPLAINED_WARNING=1` — exits `1` with borg's `--show-rc` footer as its only warning, the case where borg reports a warning status without saying why.
- `MOCK_BORG_SIMULATE_IGNORED_ERROR=1` — exits `1` with an `ERROR` record as its only diagnostic, for the case where a file change pattern would otherwise suppress it.
- `MOCK_BORG_FATAL_UNRELATED=1` — simulates a `file changed` warning alongside an unrelated fatal repository error, exiting `2`.
- `MOCK_BORG_EXIT_SUBCOMMAND=<subcommand>` — ends that subcommand at once with `MOCK_BORG_EXIT_CODE` (default `2`) and a line on stderr, so a fatal or warning status of one step can be tested on its own.
- `MOCK_BORG_LIST_SHORT=<name>` — what `list --short` prints as the newest archive (default `test-host-2024-01-01T00:00:00`); empty for a repository without a matching archive.
- `MOCK_BORG_EXTRACT_CREATE=1` — recreates each selected path under the working directory; `MOCK_BORG_EXTRACT_CONTENT` sets what those files contain.
- `BORG_BINARY=/path/to/mock` — override the borg binary path in tests.

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->
