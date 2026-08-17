// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { SegmentedOption } from '../components/BaseSegmented.vue'

/** Shared by the storage-usage and backup-size trend widgets. */
export const STORAGE_TREND_RANGE_OPTIONS: SegmentedOption<number>[] = [
  { value: 14, label: '14d' },
  { value: 30, label: '30d' },
  { value: 90, label: '90d' },
  { value: 365, label: '1y' },
]
