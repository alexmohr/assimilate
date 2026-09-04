// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type { ScheduleFailureAction, ScheduleRow, ScheduleType } from '../types/schedule'
import type { ReportRow } from '../types/report'
import type {
  DeleteFailedReportsResponse,
  FailedReportCountResponse,
  HookCommand,
  ScheduleBackupSourcesResponse,
  ScheduleTargetResponse,
  HealthSummaryResponse,
} from '../types/generated'

export interface ScheduleAgentBackupSourcesOverride {
  agent_id: number
  paths: string[]
}

export interface ScheduleAgentTextOverride {
  agent_id: number
  raw_text: string
}

export interface ScheduleAgentCommandsOverride {
  agent_id: number
  pre_backup_commands: HookCommand[]
  post_backup_commands: HookCommand[]
}

export interface CreateScheduleRequest {
  name: string
  cron_expression: string
  enabled: boolean
  canary_enabled: boolean
  exclude_patterns_raw: string
  file_change_patterns_raw: string
  ignore_global_excludes: boolean
  keep_hourly: number
  keep_daily: number
  keep_weekly: number
  keep_monthly: number
  keep_yearly: number
  compact_enabled: boolean
  rate_limit_kbps: number
  pre_backup_commands: HookCommand[]
  post_backup_commands: HookCommand[]
  hook_timeout_seconds: number
  missed_backup_threshold: number
  backup_sources: string[]
  backup_sources_per_agent?: ScheduleAgentBackupSourcesOverride[]
  exclude_patterns_per_agent?: ScheduleAgentTextOverride[]
  file_change_patterns_per_agent?: ScheduleAgentTextOverride[]
  commands_per_agent?: ScheduleAgentCommandsOverride[]
  agent_ids: number[]
  repo_id: number
  schedule_type: ScheduleType
  on_failure: ScheduleFailureAction
}

// The detail page's save flow builds this incrementally (only the fields
// relevant to the current per-agent override toggles get set) and never
// includes `schedule_type` - it isn't editable after creation. The schedules
// list's inline enable/disable toggle sends an even smaller subset.
//
// `cron_expression` and `enabled` stay required: the backend's
// UpdateScheduleRequest declares `cron_expression` as a non-optional String
// (omitting it fails cron validation) and defaults a missing `enabled` to
// `true` (omitting it would silently re-enable a disabled schedule).
export type UpdateScheduleRequest = Pick<CreateScheduleRequest, 'cron_expression' | 'enabled'> &
  Partial<Omit<CreateScheduleRequest, 'schedule_type' | 'cron_expression' | 'enabled'>>

export interface RunScheduleRequest {
  agent_ids?: number[]
}

export async function listSchedules(options?: { timeout?: number }): Promise<ScheduleRow[]> {
  const response = await apiClient.get<ScheduleRow[]>('/schedules', {
    timeout: options?.timeout,
  })
  return response.data
}

export async function getSchedule(id: number | string): Promise<ScheduleRow> {
  const response = await apiClient.get<ScheduleRow>(`/schedules/${id}`)
  return response.data
}

export async function createSchedule(data: CreateScheduleRequest): Promise<ScheduleRow> {
  const response = await apiClient.post<ScheduleRow>('/schedules', data)
  return response.data
}

export async function updateSchedule(
  id: number | string,
  data: UpdateScheduleRequest,
): Promise<ScheduleRow> {
  const response = await apiClient.put<ScheduleRow>(`/schedules/${id}`, data)
  return response.data
}

export async function deleteSchedule(id: number | string): Promise<void> {
  await apiClient.delete(`/schedules/${id}`)
}

export async function runSchedule(
  id: number | string,
  data: RunScheduleRequest = {},
): Promise<void> {
  await apiClient.post(`/schedules/${id}/run`, data)
}

export async function cancelSchedule(id: number | string): Promise<void> {
  await apiClient.post(`/schedules/${id}/cancel`)
}

export async function listScheduleTargets(id: number | string): Promise<ScheduleTargetResponse[]> {
  const response = await apiClient.get<ScheduleTargetResponse[]>(`/schedules/${id}/targets`)
  return response.data
}

export async function getScheduleBackupSources(
  id: number | string,
): Promise<ScheduleBackupSourcesResponse> {
  const response = await apiClient.get<ScheduleBackupSourcesResponse>(`/schedules/${id}/sources`)
  return response.data
}

export async function listScheduleReports(
  id: number | string,
  limit?: number,
): Promise<ReportRow[]> {
  const response = await apiClient.get<ReportRow[]>(`/schedules/${id}/reports`, {
    params: { limit },
  })
  return response.data
}

export async function deleteFailedScheduleReports(
  id: number | string,
): Promise<DeleteFailedReportsResponse> {
  const response = await apiClient.delete<DeleteFailedReportsResponse>(
    `/schedules/${id}/reports/failed`,
  )
  return response.data
}

/**
 * Unbounded by the report list's own pagination window - the true count a
 * "clean up failed backups" confirmation is about to delete.
 */
export async function countFailedScheduleReports(id: number | string): Promise<number> {
  const response = await apiClient.get<FailedReportCountResponse>(
    `/schedules/${id}/reports/failed/count`,
  )
  return response.data.count
}

export async function listRepoSchedules(repoId: number | string): Promise<ScheduleRow[]> {
  const response = await apiClient.get<ScheduleRow[]>(`/repos/${repoId}/schedules`)
  return response.data
}

export async function getScheduleHealth(options?: {
  timeout?: number
}): Promise<HealthSummaryResponse[]> {
  const response = await apiClient.get<HealthSummaryResponse[]>('/stats/health', {
    timeout: options?.timeout,
  })
  return response.data
}
