// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  changeUserPassword,
  createUser,
  deleteUser,
  getUserGroups,
  getUserPermissions,
  getUserRoles,
  listUsers,
  updateUserGroups,
  updateUserRole,
  updateUserRoles,
} from './users'

describe('users api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('lists users', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listUsers()

    expect(apiClient.get).toHaveBeenCalledWith('/users')
  })

  it('creates a user', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await createUser({ username: 'alice', password: 'hunter22', role: 'user' })

    expect(apiClient.post).toHaveBeenCalledWith('/users', {
      username: 'alice',
      password: 'hunter22',
      role: 'user',
    })
  })

  it('deletes a user', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({})

    await deleteUser(5)

    expect(apiClient.delete).toHaveBeenCalledWith('/users/5')
  })

  it('gets user roles', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await getUserRoles(5)

    expect(apiClient.get).toHaveBeenCalledWith('/users/5/roles')
  })

  it('gets user groups', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await getUserGroups(5)

    expect(apiClient.get).toHaveBeenCalledWith('/users/5/groups')
  })

  it('gets user permissions', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await getUserPermissions(5)

    expect(apiClient.get).toHaveBeenCalledWith('/users/5/permissions')
  })

  it('updates a user role', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({})

    await updateUserRole(5, { role: 'admin' })

    expect(apiClient.put).toHaveBeenCalledWith('/users/5/role', { role: 'admin' })
  })

  it('changes a user password', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({})

    await changeUserPassword(5, { password: 'newpassword1' })

    expect(apiClient.put).toHaveBeenCalledWith('/users/5/password', {
      password: 'newpassword1',
    })
  })

  it('updates user roles', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({})

    await updateUserRoles(5, { role_ids: [1, 2] })

    expect(apiClient.put).toHaveBeenCalledWith('/users/5/roles', { role_ids: [1, 2] })
  })

  it('updates user groups', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({})

    await updateUserGroups(5, { group_ids: [1, 2] })

    expect(apiClient.put).toHaveBeenCalledWith('/users/5/groups', { group_ids: [1, 2] })
  })
})
