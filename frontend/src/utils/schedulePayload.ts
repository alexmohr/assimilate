// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { dropBlankCommands, parseLines } from './validation'
import type { CreateScheduleRequest } from '../api/schedules'
import type { ScheduleFormState } from '../types/scheduleForm'

/**
 * The part of a create/update request that comes from the schedule form
 * alone. What the two callers add on top differs - the wizard supplies the
 * hosts, targets and type a new schedule needs, the detail page's Save
 * supplies per-agent overrides - but every field below is derived from the
 * same form state, and having it written out twice is how the two screens
 * drift apart.
 */
export type ScheduleFormPayload = Omit<
  CreateScheduleRequest,
  'agent_ids' | 'repo_id' | 'repo_targets' | 'schedule_type' | 'on_failure'
>

export function scheduleFormPayload(form: ScheduleFormState): ScheduleFormPayload {
  return {
    name: form.name,
    cron_expression: form.cron_expression,
    enabled: form.enabled,
    canary_enabled: form.canary_enabled,
    vm_snapshot_enabled: form.vm_snapshot_enabled,
    exclude_patterns_raw: form.exclude_patterns,
    file_change_patterns_raw: form.file_change_patterns,
    ignore_global_excludes: form.ignore_global_excludes,
    keep_hourly: form.keep_hourly,
    keep_daily: form.keep_daily,
    keep_weekly: form.keep_weekly,
    keep_monthly: form.keep_monthly,
    keep_yearly: form.keep_yearly,
    compact_enabled: form.compact_enabled,
    rate_limit_kbps: form.rate_limit_kbps,
    pre_backup_commands: dropBlankCommands(form.pre_backup_commands),
    post_backup_commands: dropBlankCommands(form.post_backup_commands),
    hook_timeout_seconds: form.hook_timeout_seconds,
    missed_backup_threshold: form.missed_backup_threshold,
    wake_override: form.wake_override,
    catch_up_missed_runs: form.catch_up_missed_runs,
    catch_up_min_lead_minutes: form.catch_up_min_lead_minutes,
    backup_sources: parseLines(form.backup_sources),
  }
}
