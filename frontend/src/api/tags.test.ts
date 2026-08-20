// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import { createTag, listEntityTags, listTags, setEntityTags } from './tags'

describe('tags api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('lists tags in a scope', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listTags('repo')

    expect(apiClient.get).toHaveBeenCalledWith('/tags', { params: { scope: 'repo' } })
  })

  it('lists tags in the host scope', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listTags('host')

    expect(apiClient.get).toHaveBeenCalledWith('/tags', { params: { scope: 'host' } })
  })

  it('lists the tags assigned to an entity', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listEntityTags('/repos/12')

    expect(apiClient.get).toHaveBeenCalledWith('/repos/12/tags')
  })

  it('sets the tags assigned to an entity', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({})

    await setEntityTags('/repos/12', [1, 2, 3])

    expect(apiClient.put).toHaveBeenCalledWith('/repos/12/tags', { tag_ids: [1, 2, 3] })
  })

  it('creates a tag', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({ data: {} })

    await createTag('nightly', '#ff8800', 'repo')

    expect(apiClient.post).toHaveBeenCalledWith('/tags', {
      name: 'nightly',
      color: '#ff8800',
      scope: 'repo',
    })
  })
})
