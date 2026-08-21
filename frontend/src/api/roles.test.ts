// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import { createRole, deleteRole, listRoles, updateRole } from './roles'
import type { CreateRoleRequest, RolePermissions, UpdateRoleRequest } from './roles'

const permissions: RolePermissions = {
  can_create_agent: true,
  can_delete_agent: false,
  can_delete_own_agent: false,
  can_create_repo: true,
  can_delete_repo: false,
  can_delete_own_repo: false,
  can_create_schedule: true,
  can_delete_schedule: false,
  can_delete_own_schedule: false,
  can_manage_tags: false,
  can_view_all_repos: false,
  can_manage_tunnels: false,
  can_upgrade_agent: false,
}

describe('roles api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('lists roles', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })

    await listRoles()

    expect(apiClient.get).toHaveBeenCalledWith('/roles')
  })

  it('creates a role', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})
    const data: CreateRoleRequest = { name: 'Operator', ...permissions }

    await createRole(data)

    expect(apiClient.post).toHaveBeenCalledWith('/roles', data)
  })

  it('updates a role', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({})
    const data: UpdateRoleRequest = { name: 'Operator', ...permissions }

    await updateRole(4, data)

    expect(apiClient.put).toHaveBeenCalledWith('/roles/4', data)
  })

  it('deletes a role', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({})

    await deleteRole(4)

    expect(apiClient.delete).toHaveBeenCalledWith('/roles/4')
  })
})
