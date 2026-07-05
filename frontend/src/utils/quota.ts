// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { QuotaAction } from '../types/generated'

const BYTES_PER_GB = 1073741824

const QUOTA_ACTION_LABELS: Record<QuotaAction, string> = {
  notify_only: 'Notify only',
  block_backups: 'Block backups',
  disable_schedule: 'Disable schedule',
}

export function actionLabel(action: QuotaAction): string {
  return QUOTA_ACTION_LABELS[action]
}

export function bytesToGb(bytes: number): number {
  return Math.round((bytes / BYTES_PER_GB) * 100) / 100
}

/** Entering 0 (or a negative value) persists a 0-byte threshold, it does not clear it. */
export function gbToBytes(gb: number): number {
  return Math.round(gb * BYTES_PER_GB)
}
