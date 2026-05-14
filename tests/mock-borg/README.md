# mock-borg

Small test double for `borg` used by integration tests.

## Environment variables

- `MOCK_BORG_LOG=/path/to/log` — logs every invocation, one line per call.
- `MOCK_BORG_FAIL=1` — simulates a borg connection failure and exits `2`.
- `MOCK_BORG_SIMULATE_WARNING=1` — simulates a `file changed` warning and exits `1`.
- `BORG_BINARY=/path/to/mock` — override the borg binary path in tests.

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->
