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
 * The form's starting shape: what a new schedule looks like before anything
 * is typed into it, and what a loaded schedule's fields fall back to before
 * `populateForm()` overwrites them. Exported so tests can build fixtures
 * against the same defaults instead of re-typing them.
 */
export const DEFAULT_SCHEDULE_FORM_STATE: ScheduleFormState = {
  name: '',
  cron_expression: '0 2 * * *',
  enabled: true,
  canary_enabled: true,
  exclude_patterns: '',
  file_change_patterns: '',
  ignore_global_excludes: false,
  keep_hourly: 24,
  keep_daily: 7,
  keep_weekly: 4,
  keep_monthly: 12,
  keep_yearly: 10,
  compact_enabled: true,
  rate_limit_kbps: 0,
  pre_backup_commands: '',
  post_backup_commands: '',
  backup_sources: '',
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
