// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import { cancelRestoreRun, getRestoreRun, listRestoreRuns } from './restores'
import { makeRestoreRun } from '../test-utils/restoreRun'

describe('restores api', () => {
  beforeEach(() => {
    vi.resetAllMocks()
  })

  it('lists restores, with a limit only when one is given', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [makeRestoreRun()] })

    expect(await listRestoreRuns()).toEqual([makeRestoreRun()])
    await listRestoreRuns(10)

    expect(apiClient.get).toHaveBeenNthCalledWith(1, '/restores', { params: {} })
    expect(apiClient.get).toHaveBeenNthCalledWith(2, '/restores', { params: { limit: 10 } })
  })

  it('fetches one restore', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: makeRestoreRun() })

    expect(await getRestoreRun('abc')).toEqual(makeRestoreRun())
    expect(apiClient.get).toHaveBeenCalledWith('/restores/abc')
  })

  it('cancels a waiting restore', async () => {
    const cancelled = makeRestoreRun({ status: 'cancelled' })
    vi.mocked(apiClient.post).mockResolvedValue({ data: cancelled })

    expect(await cancelRestoreRun('abc')).toEqual(cancelled)
    expect(apiClient.post).toHaveBeenCalledWith('/restores/abc/cancel')
  })
})
