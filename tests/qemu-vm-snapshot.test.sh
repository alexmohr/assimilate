#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Alexander Mohr

# Tests for scripts/hooks/qemu-vm-snapshot.sh against the libvirt mocks in
# tests/mock-virt. Run with: tests/qemu-vm-snapshot.test.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOOK="$REPO_ROOT/scripts/hooks/qemu-vm-snapshot.sh"
MOCK_BIN="$REPO_ROOT/tests/mock-virt"

failures=0
WORK=""

ok() { printf '  ok   %s\n' "$1"; }
bad() {
  printf '  FAIL %s\n' "$1"
  failures=$((failures + 1))
}

assert_file() {
  if [[ -f "$1" ]]; then ok "$2"; else bad "$2 (missing $1)"; fi
}

assert_missing() {
  if [[ ! -e "$1" ]]; then ok "$2"; else bad "$2 (unexpected $1)"; fi
}

assert_grep() {
  if grep -q "$2" "$1"; then ok "$3"; else bad "$3 (no match for '$2' in $1)"; fi
}

assert_not_grep() {
  if grep -q "$2" "$1"; then bad "$3 (unexpected '$2' in $1)"; else ok "$3"; fi
}

assert_eq() {
  if [[ "$1" == "$2" ]]; then ok "$3"; else bad "$3 (expected '$2', got '$1')"; fi
}

new_case() {
  printf '\n%s\n' "$1"
  WORK="$(mktemp -d)"
  export MOCK_VIRT_STATE="$WORK/state"
  export DEST_ROOT="$WORK/dest"
  mkdir -p "$MOCK_VIRT_STATE" "$WORK/images" "$DEST_ROOT"
  unset MOCK_VIRT_FAIL_BACKUP MOCK_VIRT_NO_QUIESCE MOCK_VIRT_FAIL_COMMIT MOCK_VIRT_JOB_FAILED
  unset FULL_INTERVAL SKIP_DOMAINS
}

define_domain() {
  # define_domain <name> <state> <disk target> <disk image>
  printf '%s\n' "$1" >>"$MOCK_VIRT_STATE/domains"
  printf '%s %s\n' "$1" "$2" >>"$MOCK_VIRT_STATE/states"
  printf 'file disk %s %s\n' "$3" "$4" >>"$MOCK_VIRT_STATE/disks-$1"
  printf 'mock guest disk\n' >"$4"
}

run_hook() {
  PATH="$MOCK_BIN:$PATH" sh "$HOOK" "$@"
}

count_lines() {
  if [[ -f "$1" ]]; then wc -l <"$1" | tr -d ' '; else printf '0'; fi
}

new_case "running qcow2 domain: first run is full, second run is incremental"
define_domain vm1 running vda "$WORK/images/vm1.qcow2"
run_hook >/dev/null
assert_file "$DEST_ROOT/vm1/domain.xml" "the domain definition is saved"
assert_file "$DEST_ROOT/vm1/vda.full.qcow2" "the full image is written"
assert_not_grep "$MOCK_VIRT_STATE/last-backup-vm1.xml" "<incremental>" "the first run asks for a full backup"
assert_eq "$(cat "$DEST_ROOT/vm1/chain.txt")" "vda vda.full.qcow2" "the chain lists the full image"
assert_eq "$(count_lines "$MOCK_VIRT_STATE/checkpoints-vm1")" "1" "one checkpoint exists"

sleep 1
run_hook >/dev/null
assert_grep "$MOCK_VIRT_STATE/last-backup-vm1.xml" "<incremental>assimilate-" "the second run asks for an incremental backup"
assert_eq "$(count_lines "$DEST_ROOT/vm1/chain.txt")" "2" "the chain grew by one increment"
assert_eq "$(count_lines "$MOCK_VIRT_STATE/checkpoints-vm1")" "2" "a second checkpoint exists"
assert_file "$DEST_ROOT/vm1/vda.full.qcow2" "the full image is kept as the base of the chain"

export FULL_INTERVAL=2
sleep 1
run_hook >/dev/null
assert_not_grep "$MOCK_VIRT_STATE/last-backup-vm1.xml" "<incremental>" "FULL_INTERVAL forces a new full backup"
assert_eq "$(cat "$DEST_ROOT/vm1/chain.txt")" "vda vda.full.qcow2" "the chain is reset to the new full image"
assert_eq "$(count_lines "$MOCK_VIRT_STATE/checkpoints-vm1")" "1" "the old checkpoints are dropped"
assert_eq "$(find "$DEST_ROOT/vm1" -name 'vda.*.qcow2' ! -name 'vda.full.qcow2' | wc -l | tr -d ' ')" "0" "the superseded increments are removed"

new_case "shut off domain: disks are copied once and skipped while unchanged"
define_domain vm2 "shut off" vda "$WORK/images/vm2.raw"
run_hook >/dev/null
assert_file "$DEST_ROOT/vm2/vda.img" "the disk is copied"
assert_eq "$(cat "$DEST_ROOT/vm2/chain.txt")" "vda vda.img" "the chain lists the copy"
printf 'marker\n' >"$DEST_ROOT/vm2/vda.img"
run_hook >/dev/null
assert_eq "$(cat "$DEST_ROOT/vm2/vda.img")" "marker" "the unchanged disk is not copied again"
printf 'changed guest disk\n' >"$WORK/images/vm2.raw"
run_hook >/dev/null
assert_eq "$(cat "$DEST_ROOT/vm2/vda.img")" "changed guest disk" "a changed disk is copied again"

new_case "running domain without incremental support: snapshot, copy, blockcommit"
define_domain vm3 running vda "$WORK/images/vm3.raw"
run_hook >/dev/null
assert_file "$DEST_ROOT/vm3/vda.img" "the disk is copied from the snapshot"
assert_grep "$MOCK_VIRT_STATE/calls.log" "snapshot-create-as --domain vm3" "an external snapshot is taken"
assert_grep "$MOCK_VIRT_STATE/calls.log" "blockcommit vm3 vda --active --pivot" "the overlay is committed back"
assert_eq "$(find "$WORK/images" -name 'vm3.raw.*' | wc -l | tr -d ' ')" "0" "the overlay file is removed"

new_case "running domain without a guest agent: the snapshot retries without quiesce"
export MOCK_VIRT_NO_QUIESCE=1
define_domain vm4 running vda "$WORK/images/vm4.raw"
run_hook >/dev/null
assert_file "$DEST_ROOT/vm4/vda.img" "the disk is copied without quiesce"
assert_grep "$MOCK_VIRT_STATE/calls.log" "snapshot-create-as --domain vm4 --name assimilate-tmp-[0-9TZ]* --disk-only --atomic --no-metadata$" "the retry drops --quiesce"

new_case "a failing backup job fails the hook"
export MOCK_VIRT_FAIL_BACKUP=1
define_domain vm5 running vda "$WORK/images/vm5.qcow2"
status=0
output="$(run_hook 2>&1)" || status=$?
assert_eq "$status" "1" "the hook exits non-zero"
printf '%s\n' "$output" >"$WORK/output.txt"
assert_grep "$WORK/output.txt" "domains that could not be snapshotted: vm5" "the failing domain is reported"

new_case "SKIP_DOMAINS leaves a domain alone"
define_domain vm6 running vda "$WORK/images/vm6.qcow2"
define_domain vm7 running vda "$WORK/images/vm7.qcow2"
export SKIP_DOMAINS="vm6"
run_hook >/dev/null
assert_missing "$DEST_ROOT/vm6" "the skipped domain has no target directory"
assert_file "$DEST_ROOT/vm7/vda.full.qcow2" "the other domain is still backed up"

new_case "an explicit domain list limits the run"
define_domain vm8 running vda "$WORK/images/vm8.qcow2"
define_domain vm9 running vda "$WORK/images/vm9.qcow2"
run_hook vm9 >/dev/null
assert_missing "$DEST_ROOT/vm8" "the domain that was not named is untouched"
assert_file "$DEST_ROOT/vm9/vda.full.qcow2" "the named domain is backed up"

printf '\n'
if [[ "$failures" -gt 0 ]]; then
  printf '%d assertion(s) failed\n' "$failures"
  exit 1
fi
printf 'all assertions passed\n'
