// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/**
 * The editable shape behind the schedule detail form. Lives here rather than
 * inline in the view so the tab components can be typed against it.
 */
export interface ScheduleFormState {
  name: string
  cron_expression: string
  enabled: boolean
  canary_enabled: boolean
  exclude_patterns: string
  file_change_patterns: string
  ignore_global_excludes: boolean
  keep_hourly: number
  keep_daily: number
  keep_weekly: number
  keep_monthly: number
  keep_yearly: number
  compact_enabled: boolean
  rate_limit_kbps: number
  pre_backup_commands: string
  post_backup_commands: string
  backup_sources: string
}

/**
 * Per-agent overrides for a multi-host schedule. Each record is keyed by agent
 * id; a missing or empty entry means "fall back to the schedule-level value".
 */
export interface ScheduleAgentOverrides {
  usePerHostExcludes: boolean
  perHostExcludes: Record<number, string>
  usePerHostFileChangePatterns: boolean
  perHostFileChangePatterns: Record<number, string>
  usePerAgentCmds: boolean
  perAgentPreCmds: Record<number, string>
  perAgentPostCmds: Record<number, string>
}
