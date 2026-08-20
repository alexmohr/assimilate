// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'

/** System-wide permission flags that make up a role. */
export interface RolePermissions {
  can_create_agent: boolean
  can_delete_agent: boolean
  can_delete_own_agent: boolean
  can_create_repo: boolean
  can_delete_repo: boolean
  can_delete_own_repo: boolean
  can_create_schedule: boolean
  can_delete_schedule: boolean
  can_delete_own_schedule: boolean
  can_manage_tags: boolean
  can_view_all_repos: boolean
  can_manage_tunnels: boolean
  can_upgrade_agent: boolean
}

export interface Role extends RolePermissions {
  id: number
  name: string
  is_seeded: boolean
}

export interface CreateRoleRequest extends RolePermissions {
  name: string
}

export type UpdateRoleRequest = RolePermissions

export async function listRoles(): Promise<Role[]> {
  const response = await apiClient.get<Role[]>('/roles')
  return response.data
}

export async function createRole(data: CreateRoleRequest): Promise<void> {
  await apiClient.post('/roles', data)
}

export async function updateRole(id: number, data: UpdateRoleRequest): Promise<void> {
  await apiClient.put(`/roles/${id}`, data)
}

export async function deleteRole(id: number): Promise<void> {
  await apiClient.delete(`/roles/${id}`)
}
