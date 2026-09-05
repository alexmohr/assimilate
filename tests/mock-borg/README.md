# mock-borg

Small test double for `borg` used by integration tests.

## Environment variables

- `MOCK_BORG_LOG=/path/to/log` — logs every invocation, one line per call.
- `MOCK_BORG_FAIL=1` — simulates a borg connection failure and exits `2`.
- `MOCK_BORG_SIMULATE_WARNING=1` — simulates a `file changed` warning and exits `1`.
- `MOCK_BORG_SIMULATE_UNEXPLAINED_WARNING=1` — exits `1` with borg's `--show-rc` footer as its only warning, the case where borg reports a warning status without saying why.
- `MOCK_BORG_FATAL_UNRELATED=1` — simulates a `file changed` warning alongside an unrelated fatal repository error, exiting `2`.
- `BORG_BINARY=/path/to/mock` — override the borg binary path in tests.

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->
