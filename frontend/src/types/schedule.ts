// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ScheduleResponse } from './generated'
export type ScheduleType = 'backup' | 'check' | 'verify'
export type ScheduleFailureAction = 'stop' | 'continue'
export type ScheduleRow = Omit<
  ScheduleResponse,
  'repo_id' | 'schedule_type' | 'keep_hourly' | 'on_failure'
> & {
  repo_id: number | null
  schedule_type: ScheduleType
  keep_hourly?: number
  on_failure: ScheduleFailureAction
}
