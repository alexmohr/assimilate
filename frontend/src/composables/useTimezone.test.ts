// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('../api/system', () => ({
  getSystemSettings: vi.fn(),
}))

import { getSystemSettings } from '../api/system'
import { getConfiguredTimezone, useTimezone } from './useTimezone'

describe('useTimezone', () => {
  beforeEach(() => {
    localStorage.clear()
    vi.mocked(getSystemSettings).mockReset()
    // `timezone` is a module-level singleton ref, so it carries state across
    // tests unless explicitly reset here.
    useTimezone().setTimezone(undefined)
  })

  it('setTimezone persists to storage and getConfiguredTimezone reflects it', () => {
    const { setTimezone } = useTimezone()

    setTimezone('Europe/Berlin')

    expect(getConfiguredTimezone()).toBe('Europe/Berlin')
    expect(localStorage.getItem('assimilate-timezone')).toBe('Europe/Berlin')
  })

  it('setTimezone(undefined) clears storage and falls back to the browser timezone', () => {
    const { setTimezone } = useTimezone()
    setTimezone('Europe/Berlin')

    setTimezone(undefined)

    expect(localStorage.getItem('assimilate-timezone')).toBeNull()
    expect(getConfiguredTimezone()).toBe(Intl.DateTimeFormat().resolvedOptions().timeZone)
  })

  it('loadFromBackend applies the timezone from system settings', async () => {
    vi.mocked(getSystemSettings).mockResolvedValue({
      retention_days: 30,
      report_retention_days: 30,
      failed_report_retention_days: 30,
      system_event_retention_days: 30,
      notification_delivery_retention_days: 30,
      timezone: 'Asia/Tokyo',
    } as Awaited<ReturnType<typeof getSystemSettings>>)

    const { loadFromBackend } = useTimezone()
    await loadFromBackend()

    expect(getConfiguredTimezone()).toBe('Asia/Tokyo')
  })

  it('loadFromBackend leaves the timezone unset when the request fails', async () => {
    vi.mocked(getSystemSettings).mockRejectedValue(new Error('network error'))

    const { loadFromBackend, timezone } = useTimezone()
    await loadFromBackend()

    expect(timezone.value).toBeUndefined()
  })
})
