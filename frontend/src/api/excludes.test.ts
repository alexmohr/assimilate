// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import { getExcludes, setExcludes } from './excludes'

describe('excludes api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('gets the global excludes', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { raw_text: 'node_modules\n__pycache__' } })

    await expect(getExcludes()).resolves.toEqual({ raw_text: 'node_modules\n__pycache__' })
    expect(apiClient.get).toHaveBeenCalledWith('/excludes')
  })

  it('sets the global excludes', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({ data: { raw_text: '*.log' } })

    await expect(setExcludes({ raw_text: '*.log' })).resolves.toEqual({ raw_text: '*.log' })

    expect(apiClient.put).toHaveBeenCalledWith('/excludes', { raw_text: '*.log' })
  })
})
