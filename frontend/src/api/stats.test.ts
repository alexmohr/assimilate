// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  dismissFinding,
  getActivity,
  getActivityByRange,
  getActivityDurationSamples,
  getCalendar,
  getDashboardOverview,
  getDashboardSummary,
  getScheduleCounts,
  getSystemEvents,
  acknowledgeSystemEvent,
  unacknowledgeSystemEvent,
  acknowledgeAllActivity,
} from './stats'

describe('stats api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('gets the activity feed with no params', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await getActivity()

    expect(apiClient.get).toHaveBeenCalledWith('/stats/activity', { params: undefined })
  })

  it('gets the activity feed with params', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await getActivity({ limit: 50, schedule_id: 7, run_id: 'run-101' })

    expect(apiClient.get).toHaveBeenCalledWith('/stats/activity', {
      params: { limit: 50, schedule_id: 7, run_id: 'run-101' },
    })
  })

  it('gets activity by date range without a repo filter', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await getActivityByRange({ days: 30 })

    expect(apiClient.get).toHaveBeenCalledWith('/stats/activity?days=30')
  })

  it('gets activity by date range with a repo filter', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await getActivityByRange({ days: 7, repo_id: 5 })

    expect(apiClient.get).toHaveBeenCalledWith('/stats/activity?days=7&repo_id=5')
  })

  it('gets activity duration samples', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await getActivityDurationSamples({ schedule_id: 7, repo_id: 3, limit: 20 })

    expect(apiClient.get).toHaveBeenCalledWith('/stats/activity?schedule_id=7&repo_id=3&limit=20')
  })

  it('gets system events', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await getSystemEvents(50)

    expect(apiClient.get).toHaveBeenCalledWith('/stats/system-events', {
      params: { limit: 50, acknowledged: undefined },
    })
  })

  it('gets system events filtered by acknowledgment state', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await getSystemEvents(50, 'unacknowledged')

    expect(apiClient.get).toHaveBeenCalledWith('/stats/system-events', {
      params: { limit: 50, acknowledged: 'unacknowledged' },
    })
  })

  it('acknowledges and unacknowledges a system event', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: undefined })
    vi.mocked(apiClient.delete).mockResolvedValue({ data: undefined })

    await acknowledgeSystemEvent(9)
    await unacknowledgeSystemEvent(9)

    expect(apiClient.post).toHaveBeenCalledWith('/stats/system-events/9/acknowledge')
    expect(apiClient.delete).toHaveBeenCalledWith('/stats/system-events/9/acknowledge')
  })

  it('acknowledges everything outstanding at once', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { backup_reports: 3, system_events: 1 },
    })

    const result = await acknowledgeAllActivity()

    expect(apiClient.post).toHaveBeenCalledWith('/stats/activity/acknowledge-all')
    expect(result).toEqual({ backup_reports: 3, system_events: 1 })
  })

  it('gets the dashboard summary', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: {} })

    await getDashboardSummary()

    expect(apiClient.get).toHaveBeenCalledWith('/stats/summary')
  })

  it('gets the dashboard overview', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: {} })

    await getDashboardOverview()

    expect(apiClient.get).toHaveBeenCalledWith('/stats/dashboard-overview', { timeout: undefined })
  })

  it('gets schedule counts by agent', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [{ agent_id: 1, count: 3 }] })

    await expect(getScheduleCounts()).resolves.toEqual([{ agent_id: 1, count: 3 }])

    expect(apiClient.get).toHaveBeenCalledWith('/stats/schedule-counts', { timeout: undefined })
  })

  it('dismisses a finding', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await dismissFinding('finding-1')

    expect(apiClient.post).toHaveBeenCalledWith('/stats/findings/finding-1/dismiss')
  })

  it('encodes the finding id when dismissing', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await dismissFinding('finding/with slash')

    expect(apiClient.post).toHaveBeenCalledWith('/stats/findings/finding%2Fwith%20slash/dismiss')
  })

  it('gets the calendar without a repo filter', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await getCalendar({ month: '2026-08' })

    expect(apiClient.get).toHaveBeenCalledWith('/stats/calendar?month=2026-08')
  })

  it('gets the calendar with a repo filter', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await getCalendar({ month: '2026-08', repo_id: 5 })

    expect(apiClient.get).toHaveBeenCalledWith('/stats/calendar?month=2026-08&repo_id=5')
  })
})
