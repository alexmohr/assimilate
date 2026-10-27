// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { FsSnapshotMode } from '../types/generated'
import type { ScheduleFormState } from '../types/scheduleForm'

/**
 * Whether a schedule's backups snapshot their sources at all, mirroring
 * `FsSnapshotMode::is_on` on the server. A schedule loaded without the
 * field, from a server older than the setting, reads as off.
 */
export function isFsSnapshotOn(mode: FsSnapshotMode | undefined): boolean {
  switch (mode) {
    case 'required':
    case 'best_effort':
      return true
    case 'off':
    case undefined:
      return false
  }
}

/**
 * Whether `percent` is an LVM snapshot size the server takes, mirroring
 * `is_valid_lvm_snapshot_percent`: a whole percent from 1 to 100.
 */
export function isValidLvmSnapshotPercent(percent: number): boolean {
  return Number.isInteger(percent) && percent >= 1 && percent <= 100
}

/**
 * What a schedule form still needs before its LVM snapshot size can be
 * saved, or null. The size only counts while snapshots are on; a cleared
 * field holds no number at all, which this also catches.
 */
export function lvmSnapshotPercentProblem(
  form: Pick<ScheduleFormState, 'fs_snapshot' | 'lvm_snapshot_percent'>,
): string | null {
  return isFsSnapshotOn(form.fs_snapshot) && !isValidLvmSnapshotPercent(form.lvm_snapshot_percent)
    ? 'an LVM snapshot size from 1 to 100'
    : null
}
