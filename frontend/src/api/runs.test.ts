// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import { getRunEvents } from './runs'

describe('runs api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
  })

  it('gets the events for a run', async () => {
    const events = [
      {
        id: 1,
        run_id: 'run-1',
        target: 'source',
        event_type: 'wake_sent',
        message: 'Sent Wake-on-LAN',
        occurred_at: '2026-08-28T12:00:00Z',
      },
    ]
    vi.mocked(apiClient.get).mockResolvedValue({ data: events })

    await expect(getRunEvents('run-1')).resolves.toEqual(events)

    expect(apiClient.get).toHaveBeenCalledWith('/runs/run-1/events')
  })
})
