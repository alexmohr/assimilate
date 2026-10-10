// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { RestoreRun, RestoreRunStatus } from '../types/generated'
import type { BadgeTone } from './badge'

/** Whether the restore is over, so no further change to it will come. */
export function isRestoreFinished(status: RestoreRunStatus): boolean {
  switch (status) {
    case 'pending':
    case 'running':
      return false
    case 'success':
    case 'failed':
    case 'cancelled':
      return true
  }
}

export function restoreStatusLabel(status: RestoreRunStatus): string {
  switch (status) {
    case 'pending':
      return 'Waiting for agent'
    case 'running':
      return 'Restoring'
    case 'success':
      return 'Restored'
    case 'failed':
      return 'Failed'
    case 'cancelled':
      return 'Cancelled'
  }
}

export function restoreStatusTone(status: RestoreRunStatus): BadgeTone {
  switch (status) {
    case 'pending':
      return 'neutral'
    case 'running':
      return 'info'
    case 'success':
      return 'success'
    case 'failed':
      return 'danger'
    case 'cancelled':
      return 'neutral'
  }
}

/** What the restore covers: the requested paths, or the whole archive. */
export function restoreScope(run: Pick<RestoreRun, 'paths'>): string {
  return run.paths.length > 0 ? run.paths.join(', ') : 'the whole archive'
}
