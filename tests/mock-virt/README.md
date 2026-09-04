# mock-virt

Test doubles for `virsh` and `qemu-img`, used by the agent's tests in `crates/agent/src/vm.rs` to exercise virtual-machine staging without libvirt.

## State directory

`MOCK_VIRT_STATE` points at a directory describing the fake host. The mocks read and update it:

- `domains` — one domain name per line, returned by `virsh list`.
- `states` — `<domain> <state>` per line, returned by `virsh domstate`.
- `disks-<domain>` — `<type> <device> <target> <source>` per line, returned by `virsh domblklist`.
- `checkpoints-<domain>` — checkpoints created so far, maintained by `backup-begin` and `checkpoint-delete`.
- `calls.log` — every mock invocation, one line per call.
- `last-backup-<domain>.xml`, `last-checkpoint-<domain>.xml` — the XML of the most recent `backup-begin`.
- `last-defined.xml` — the definition the most recent `define` was given, so a test can assert how a restored domain was rewritten.

`qemu-img info` reports `qcow2` for paths ending in `.qcow2` and `raw` for everything else.

## Environment variables

- `MOCK_VIRT_NO_BACKUP_BEGIN=1` — pretends libvirt has no `backup-begin` command.
- `MOCK_VIRT_FAIL_BACKUP=1` — makes `backup-begin` fail.
- `MOCK_VIRT_JOB_FAILED=1` — reports the completed backup job as failed.
- `MOCK_VIRT_NO_QUIESCE=1` — makes `snapshot-create-as --quiesce` fail, as on a host without a guest agent.
- `MOCK_VIRT_FAIL_COMMIT=1` — makes `blockcommit` fail.
- `MOCK_VIRT_FAIL_DEFINE=1` — makes `define` fail, as when a restored definition is rejected.
- `MOCK_VIRT_FAIL_START=1` — makes `start` fail.
- `MOCK_VIRT_BACKUP_KIB=<n>` — size in KiB of every image `backup-begin` writes (default 4).

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->
