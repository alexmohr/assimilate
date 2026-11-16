// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { isFsSnapshotOn, isValidLvmSnapshotPercent } from './fsSnapshot'
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
    database_backup_enabled: form.database_backup_enabled,
    fs_snapshot: form.fs_snapshot,
    // The size is hidden while snapshots are off; one typed out of range
    // before could not be fixed there, so the server keeps what it has.
    ...(isFsSnapshotOn(form.fs_snapshot) || isValidLvmSnapshotPercent(form.lvm_snapshot_percent)
      ? { lvm_snapshot_percent: form.lvm_snapshot_percent }
      : {}),
    files_mode: form.files_mode,
    run_host_commands: form.run_host_commands,
    exclude_patterns_raw: form.exclude_patterns,
    include_patterns_raw: form.include_patterns,
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
  const payload: Partial<ScheduleFormPayload> = overrides.clearPerHost
    ? {
        exclude_patterns_per_agent: [],
        include_patterns_per_agent: [],
        file_change_patterns_per_agent: [],
        commands_per_agent: [],
      }
    : {}

  if (overrides.usePerHostExcludes) {
    payload.exclude_patterns_raw = ''
    payload.exclude_patterns_per_agent = agentIds.map((agent_id) => ({
      agent_id,
      raw_text: overrides.perHostExcludes[agent_id] ?? '',
    }))
  }

  if (overrides.usePerHostIncludes) {
    payload.include_patterns_raw = ''
    payload.include_patterns_per_agent = agentIds.map((agent_id) => ({
      agent_id,
      raw_text: overrides.perHostIncludes[agent_id] ?? '',
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

/**
 * The repository a schedule counts as its primary target.
 *
 * Mirrors the server's `primary_target`: the first *required* target, falling
 * back to the first written one only when nothing is required. The server
 * recomputes this from `repo_targets` whenever that field is sent, so the
 * `repo_id` the client sends alongside it is inert today - but sending the
 * first *written* target instead meant the two disagreed for any list that
 * starts with a best-effort target, and would have picked the wrong
 * repository the moment any path trusted the client's value.
 */
export function primaryRepoId(targets: readonly ScheduleRepoTarget[]): number {
  return (targets.find((target) => target.required) ?? targets[0]).repo_id
}

/** A schedule's own paths as the Sources pane edits them: one shared list, or one list per host. */
export interface OwnPaths {
  usePerHostPaths: boolean
  perHostSources: Record<number, string>
  /** The schedule has per-host paths saved that shared paths must delete. */
  clearSavedPerHost?: boolean
}

/**
 * The paths half of a create/update request. Paths split per host replace
 * the shared list rather than adding to it, and a host given none is left
 * out rather than sent an empty list. Turning the split off on a schedule
 * that saved per-host paths (`clearSavedPerHost`) sends an empty per-host
 * list to delete them: left in place, they would still win over the shared
 * paths. Otherwise the per-host field is left out and keeps what is stored.
 */
export function ownPathsPayload(
  form: ScheduleFormState,
  own: OwnPaths,
  agentIds: readonly number[],
): Pick<ScheduleFormPayload, 'backup_sources' | 'backup_sources_per_agent'> {
  if (!own.usePerHostPaths) {
    const shared = parseLines(form.backup_sources)
    return own.clearSavedPerHost
      ? { backup_sources: shared, backup_sources_per_agent: [] }
      : { backup_sources: shared }
  }
  return {
    backup_sources: [],
    backup_sources_per_agent: agentIds
      .map((agent_id) => ({ agent_id, paths: parseLines(own.perHostSources[agent_id] ?? '') }))
      .filter((entry) => entry.paths.length > 0),
  }
}

/**
 * Why a backup schedule's sources could never back anything up, if they
 * could not: the same rule the server applies on create and on edit.
 *
 * `hostPaths` gives a host's own paths, where they are known. With it, a
 * schedule taking each host's files needs every host to list some, as the
 * server checks on create; without it, that is left to the server.
 */
export function sourcesProblem(
  form: ScheduleFormState,
  own: OwnPaths,
  agentIds: readonly number[],
  hostPaths?: (agentId: number) => readonly string[] | undefined,
): string | null {
  if (form.files_mode === 'host') {
    const bare = agentIds.some((id) => hostPaths?.(id)?.length === 0)
    return bare ? "paths on every host, or its own files: a host's Sources list none" : null
  }
  if (form.files_mode === 'own') {
    const paths = ownPathsPayload(form, own, agentIds)
    const any =
      (paths.backup_sources?.length ?? 0) > 0 || (paths.backup_sources_per_agent?.length ?? 0) > 0
    return any ? null : 'at least one backup path for its own files'
  }
  if (form.files_mode === 'off' && !form.vm_snapshot_enabled && !form.database_backup_enabled) {
    return 'virtual machines or databases, since it reads no files'
  }
  return null
}
