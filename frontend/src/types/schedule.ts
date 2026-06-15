// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

export type ScheduleType = 'backup' | 'check' | 'verify'

export type ScheduleFailureAction = 'stop' | 'continue'

export interface ScheduleRow {
  id: number
  repo_id: number | null
  name: string
  schedule_type: ScheduleType
  cron_expression: string
  enabled: boolean
  canary_enabled: boolean
  last_run_at: string | null
  next_run_at: string | null
  exclude_patterns_raw?: string
  exclude_patterns?: string[]
  ignore_global_excludes: boolean
  keep_hourly?: number
  keep_daily: number
  keep_weekly: number
  keep_monthly: number
  keep_yearly: number
  compact_enabled: boolean
  pre_backup_commands: string
  post_backup_commands: string
  on_failure?: ScheduleFailureAction
  execution_mode?: string
  target_hostnames: string[]
}
