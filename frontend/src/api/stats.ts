// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type { DashboardOverview } from '../types/dashboard'
import type {
  CalendarDayResponse,
  DashboardSummaryResponse,
  ScheduleCountByAgentResponse,
} from '../types/generated'

// NOTE: /stats/health is intentionally not covered by this module - it is
// owned by a separate schedules module and call sites keep the raw
// apiClient.get('/stats/health') call.

export interface ActivityEntry {
  id: number
  hostname: string
  target_name: string
  started_at: string
  finished_at: string
  status: string
  duration_secs: number
  repo_id: number | null
  archive_name: string | null
  error_message: string | null
  schedule_id: number | null
  schedule_name: string | null
  run_id: string | null
  acknowledged: boolean
}

export interface ActivityFeedParams {
  limit?: number
  schedule_id?: number
  run_id?: string
}

export async function getActivity(params?: ActivityFeedParams): Promise<ActivityEntry[]> {
  const response = await apiClient.get<ActivityEntry[]>('/stats/activity', { params })
  return response.data
}

export interface ActivityByRangeParams {
  days: number
  repo_id?: number
}

export async function getActivityByRange(params: ActivityByRangeParams): Promise<ActivityEntry[]> {
  const searchParams = new URLSearchParams({ days: String(params.days) })
  if (params.repo_id !== undefined) {
    searchParams.set('repo_id', String(params.repo_id))
  }
  const response = await apiClient.get<ActivityEntry[]>(
    `/stats/activity?${searchParams.toString()}`,
  )
  return response.data
}

export interface ActivityDurationSample {
  status: string
  duration_secs: number
}

export interface ActivityDurationSampleParams {
  schedule_id: number
  repo_id: number
  limit: number
}

export async function getActivityDurationSamples(
  params: ActivityDurationSampleParams,
): Promise<ActivityDurationSample[]> {
  const searchParams = new URLSearchParams({
    schedule_id: String(params.schedule_id),
    repo_id: String(params.repo_id),
    limit: String(params.limit),
  })
  const response = await apiClient.get<ActivityDurationSample[]>(
    `/stats/activity?${searchParams.toString()}`,
  )
  return response.data
}

export interface SystemEventEntry {
  id: number
  created_at: string
  event_type: string
  hostname: string | null
  message: string
}

export async function getSystemEvents(limit: number): Promise<SystemEventEntry[]> {
  const response = await apiClient.get<SystemEventEntry[]>('/stats/system-events', {
    params: { limit },
  })
  return response.data
}

export type DashboardSummary = DashboardSummaryResponse

export async function getDashboardSummary(): Promise<DashboardSummary> {
  const response = await apiClient.get<DashboardSummary>('/stats/summary')
  return response.data
}

export async function getDashboardOverview(options?: {
  timeout?: number
}): Promise<DashboardOverview> {
  const response = await apiClient.get<DashboardOverview>('/stats/dashboard-overview', {
    timeout: options?.timeout,
  })
  return response.data
}

export async function getScheduleCounts(options?: {
  timeout?: number
}): Promise<ScheduleCountByAgentResponse[]> {
  const response = await apiClient.get<ScheduleCountByAgentResponse[]>('/stats/schedule-counts', {
    timeout: options?.timeout,
  })
  return response.data
}

export async function dismissFinding(findingId: string): Promise<void> {
  await apiClient.post(`/stats/findings/${encodeURIComponent(findingId)}/dismiss`)
}

export async function acknowledgeActivityEntry(id: number): Promise<void> {
  await apiClient.post(`/stats/activity/${id}/acknowledge`)
}

export async function unacknowledgeActivityEntry(id: number): Promise<void> {
  await apiClient.delete(`/stats/activity/${id}/acknowledge`)
}

export interface CalendarParams {
  month: string
  repo_id?: number
}

export async function getCalendar(params: CalendarParams): Promise<CalendarDayResponse[]> {
  const searchParams = new URLSearchParams({ month: params.month })
  if (params.repo_id !== undefined) {
    searchParams.set('repo_id', String(params.repo_id))
  }
  const response = await apiClient.get<CalendarDayResponse[]>(
    `/stats/calendar?${searchParams.toString()}`,
  )
  return response.data
}
