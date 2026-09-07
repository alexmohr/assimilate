// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { dropBlankCommands, parseLines } from './validation'
import type { CreateScheduleRequest, ScheduleRepoTarget } from '../api/schedules'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'

/**
 * The part of a create/update request that comes from the schedule form
 * alone. What the two callers add on top differs - the wizard supplies the
 * hosts, targets and type a new schedule needs - but every field below is
 * derived from the same form state, and having it written out twice is how
 * the two screens drift apart.
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

/**
 * The per-agent half of a create/update request, from the Advanced step's
 * override state.
 *
 * Each override *replaces* its schedule-wide counterpart rather than adding
 * to it, which is why turning one on also clears the shared field.
 *
 * Shared by the wizard and the detail page's Save for the reason the form
 * payload is: the wizard offers the same Advanced step, and building this
 * on only one of the two screens is how a schedule created with per-host
 * excludes silently came out without them.
 */
export function agentOverridePayload(
  overrides: ScheduleAgentOverrides,
  agentIds: readonly number[],
): Partial<ScheduleFormPayload> {
  const payload: Partial<ScheduleFormPayload> = {}

  if (overrides.usePerHostExcludes) {
    payload.exclude_patterns_raw = ''
    payload.exclude_patterns_per_agent = agentIds.map((agent_id) => ({
      agent_id,
      raw_text: overrides.perHostExcludes[agent_id] ?? '',
    }))
  }

  if (overrides.usePerHostFileChangePatterns) {
    payload.file_change_patterns_raw = ''
    payload.file_change_patterns_per_agent = agentIds.map((agent_id) => ({
      agent_id,
      raw_text: overrides.perHostFileChangePatterns[agent_id] ?? '',
    }))
  }

  if (overrides.usePerAgentCmds) {
    payload.pre_backup_commands = []
    payload.post_backup_commands = []
    payload.commands_per_agent = agentIds.map((agent_id) => ({
      agent_id,
      pre_backup_commands: dropBlankCommands(overrides.perAgentPreCmds[agent_id] ?? []),
      post_backup_commands: dropBlankCommands(overrides.perAgentPostCmds[agent_id] ?? []),
    }))
  }

  return payload
}

/**
 * What is missing from a schedule's target list, or `null` when it is usable.
 *
 * The rule mirrors the server's `resolve_repo_targets`: at least one target,
 * at least one of them required, or a run could report success having written
 * nothing. Shared so the wizard's step blocker and the detail page's save
 * guard cannot drift apart from each other or from the server.
 */
export function repoTargetsProblem(targets: readonly ScheduleRepoTarget[]): string | null {
  if (targets.length === 0) return 'at least one repository'
  if (!targets.some((target) => target.required)) return 'at least one required repository'
  return null
}
