// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { FsSnapshotMode } from '../types/generated'

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
