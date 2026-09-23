// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import { agentAvailabilityApi, repoAvailabilityApi } from './availability'

const AVAILABILITY = {
  intermittent: true,
  catch_up_recheck_minutes: 15,
  catch_up_give_up_minutes: 4320,
  waiting: [],
}

describe('availability api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
  })

  it("loads a repository's availability", async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: AVAILABILITY })

    await expect(repoAvailabilityApi(7).load()).resolves.toEqual(AVAILABILITY)

    expect(apiClient.get).toHaveBeenCalledWith('/repos/7/availability')
  })

  it("saves a repository's availability with its re-check interval", async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: AVAILABILITY })
    const update = {
      intermittent: true,
      catch_up_recheck_minutes: 15,
      catch_up_give_up_minutes: 4320,
    }

    await expect(repoAvailabilityApi(7).save(update)).resolves.toEqual(AVAILABILITY)

    expect(apiClient.put).toHaveBeenCalledWith('/repos/7/availability', update)
  })

  it('asks a repository host whether it is back', async () => {
    const outcome = { probed: 1, reachable: 1, started: 1, abandoned: 0, dropped: 0 }
    vi.mocked(apiClient.post).mockResolvedValue({ data: outcome })

    await expect(repoAvailabilityApi(7).check?.()).resolves.toEqual(outcome)

    expect(apiClient.post).toHaveBeenCalledWith('/repos/7/availability/check')
  })

  it("loads an agent's availability within its domain", async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: AVAILABILITY })

    await expect(agentAvailabilityApi('web-01', 'lan').load()).resolves.toEqual(AVAILABILITY)

    expect(apiClient.get).toHaveBeenCalledWith('/agents/web-01/availability', {
      params: { domain: 'lan' },
    })
  })

  /** An agent reconnects on its own, so a re-check interval is never sent for one. */
  it("saves an agent's availability without a re-check interval", async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: AVAILABILITY })

    await agentAvailabilityApi('web-01').save({
      intermittent: true,
      catch_up_recheck_minutes: 15,
      catch_up_give_up_minutes: 60,
    })

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/web-01/availability',
      { intermittent: true, catch_up_give_up_minutes: 60 },
      { params: {} },
    )
  })

  it('offers no check for an agent', () => {
    expect(agentAvailabilityApi('web-01').check).toBeUndefined()
  })
})
