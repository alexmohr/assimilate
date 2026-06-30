// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { ScheduleResponse } from './generated'
export type ScheduleType = 'backup' | 'check' | 'verify'
export type ScheduleFailureAction = 'stop' | 'continue'
export type ScheduleRow = Omit<
  ScheduleResponse,
  | 'id'
  | 'repo_id'
  | 'keep_hourly'
  | 'keep_daily'
  | 'keep_weekly'
  | 'keep_monthly'
  | 'keep_yearly'
  | 'last_run_at'
  | 'next_run_at'
  | 'schedule_type'
> & {
  id: number
  repo_id: number | null
  schedule_type: ScheduleType
  keep_hourly?: number
  keep_daily: number
  keep_weekly: number
  keep_monthly: number
  keep_yearly: number
  last_run_at: string | null
  next_run_at: string | null
}
