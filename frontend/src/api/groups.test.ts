// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  createGroup,
  deleteGroup,
  listGroupMembers,
  listGroups,
  updateGroup,
  updateGroupMembers,
} from './groups'

describe('groups api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('lists groups', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listGroups()

    expect(apiClient.get).toHaveBeenCalledWith('/groups')
  })

  it('creates a group', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await createGroup({ name: 'Ops', description: 'Operations team' })

    expect(apiClient.post).toHaveBeenCalledWith('/groups', {
      name: 'Ops',
      description: 'Operations team',
    })
  })

  it('updates a group', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({})

    await updateGroup(3, { name: 'Ops', description: null })

    expect(apiClient.put).toHaveBeenCalledWith('/groups/3', { name: 'Ops', description: null })
  })

  it('deletes a group', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({})

    await deleteGroup(3)

    expect(apiClient.delete).toHaveBeenCalledWith('/groups/3')
  })

  it('lists group members', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [{ user_id: 1 }] })

    await listGroupMembers(3)

    expect(apiClient.get).toHaveBeenCalledWith('/groups/3/members')
  })

  it('updates group members', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({})

    await updateGroupMembers(3, { user_ids: [1, 2] })

    expect(apiClient.put).toHaveBeenCalledWith('/groups/3/members', { user_ids: [1, 2] })
  })
})
