// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  cancelSchedule,
  createSchedule,
  deleteSchedule,
  getSchedule,
  getScheduleBackupSources,
  getScheduleHealth,
  listRepoSchedules,
  listScheduleReports,
  listScheduleTargets,
  listSchedules,
  runSchedule,
  updateSchedule,
  type CreateScheduleRequest,
} from './schedules'

const CREATE_REQUEST: CreateScheduleRequest = {
  name: 'Nightly',
  cron_expression: '0 2 * * *',
  enabled: true,
  canary_enabled: true,
  exclude_patterns_raw: '',
  file_change_patterns_raw: '',
  ignore_global_excludes: false,
  keep_hourly: 24,
  keep_daily: 7,
  keep_weekly: 4,
  keep_monthly: 12,
  keep_yearly: 10,
  compact_enabled: true,
  rate_limit_kbps: 0,
  pre_backup_commands: [],
  post_backup_commands: [],
  backup_sources: ['/data'],
  agent_ids: [1, 2],
  repo_id: 5,
  schedule_type: 'backup',
  on_failure: 'stop',
}

describe('schedules api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('lists schedules', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [{ id: 7, name: 'Nightly' }] })

    await expect(listSchedules()).resolves.toEqual([{ id: 7, name: 'Nightly' }])

    expect(apiClient.get).toHaveBeenCalledWith('/schedules', { timeout: undefined })
  })

  it('gets a schedule', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { id: 7, name: 'Nightly' } })

    await expect(getSchedule(7)).resolves.toEqual({ id: 7, name: 'Nightly' })

    expect(apiClient.get).toHaveBeenCalledWith('/schedules/7')
  })

  it('gets a schedule by string id', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { id: 7, name: 'Nightly' } })

    await expect(getSchedule('7')).resolves.toEqual({ id: 7, name: 'Nightly' })

    expect(apiClient.get).toHaveBeenCalledWith('/schedules/7')
  })

  it('creates a schedule', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: { id: 1, name: 'Nightly' } })

    await expect(createSchedule(CREATE_REQUEST)).resolves.toEqual({ id: 1, name: 'Nightly' })

    expect(apiClient.post).toHaveBeenCalledWith('/schedules', CREATE_REQUEST)
  })

  it('updates a schedule', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: { id: 7, enabled: false } })

    await expect(
      updateSchedule(7, { cron_expression: '0 2 * * *', enabled: false }),
    ).resolves.toEqual({
      id: 7,
      enabled: false,
    })

    expect(apiClient.put).toHaveBeenCalledWith('/schedules/7', {
      cron_expression: '0 2 * * *',
      enabled: false,
    })
  })

  it('deletes a schedule', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({})

    await deleteSchedule(7)

    expect(apiClient.delete).toHaveBeenCalledWith('/schedules/7')
  })

  it('runs a schedule with no body by default', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await runSchedule(7)

    expect(apiClient.post).toHaveBeenCalledWith('/schedules/7/run', {})
  })

  it('runs a schedule scoped to specific agents', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await runSchedule(7, { agent_ids: [3] })

    expect(apiClient.post).toHaveBeenCalledWith('/schedules/7/run', { agent_ids: [3] })
  })

  it('cancels a schedule', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await cancelSchedule(7)

    expect(apiClient.post).toHaveBeenCalledWith('/schedules/7/cancel')
  })

  it('lists schedule targets', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [{ agent_id: 1, execution_order: 0 }] })

    await expect(listScheduleTargets(7)).resolves.toEqual([{ agent_id: 1, execution_order: 0 }])

    expect(apiClient.get).toHaveBeenCalledWith('/schedules/7/targets')
  })

  it('gets schedule backup sources', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { backup_sources: ['/data'] } })

    await expect(getScheduleBackupSources(7)).resolves.toEqual({ backup_sources: ['/data'] })

    expect(apiClient.get).toHaveBeenCalledWith('/schedules/7/sources')
  })

  it('lists schedule reports with a limit', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [{ id: 1, status: 'success' }] })

    await expect(listScheduleReports(7, 20)).resolves.toEqual([{ id: 1, status: 'success' }])

    expect(apiClient.get).toHaveBeenCalledWith('/schedules/7/reports', { params: { limit: 20 } })
  })

  it('lists schedule reports without a limit', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await expect(listScheduleReports(7)).resolves.toEqual([])

    expect(apiClient.get).toHaveBeenCalledWith('/schedules/7/reports', {
      params: { limit: undefined },
    })
  })

  it('lists a repo schedules', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [{ id: 7, name: 'Nightly' }] })

    await expect(listRepoSchedules(3)).resolves.toEqual([{ id: 7, name: 'Nightly' }])

    expect(apiClient.get).toHaveBeenCalledWith('/repos/3/schedules')
  })

  it('gets schedule health', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [{ hostname: 'web-01' }] })

    await expect(getScheduleHealth()).resolves.toEqual([{ hostname: 'web-01' }])

    expect(apiClient.get).toHaveBeenCalledWith('/stats/health', { timeout: undefined })
  })
})
