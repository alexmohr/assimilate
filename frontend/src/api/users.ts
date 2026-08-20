// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type { Role } from './roles'
import type { Group } from './groups'

export interface User {
  id: number
  username: string
  role: 'admin' | 'user'
  created_at: string
  last_login_at: string | null
}

export interface RepoPermission {
  user_id: number
  repo_id: number
  can_view: boolean
  can_backup: boolean
  can_modify_schedules: boolean
  can_extract: boolean
  can_delete: boolean
}

export interface CreateUserRequest {
  username: string
  password: string
  role: 'admin' | 'user'
}

export interface UpdateUserRoleRequest {
  role: 'admin' | 'user'
}

export interface UpdateUserPasswordRequest {
  password: string
}

export interface UpdateUserRolesRequest {
  role_ids: number[]
}

export interface UpdateUserGroupsRequest {
  group_ids: number[]
}

export async function listUsers(): Promise<User[]> {
  const response = await apiClient.get<User[]>('/users')
  return response.data
}

export async function createUser(data: CreateUserRequest): Promise<void> {
  await apiClient.post('/users', data)
}

export async function deleteUser(id: number): Promise<void> {
  await apiClient.delete(`/users/${id}`)
}

export async function getUserRoles(id: number): Promise<Role[]> {
  const response = await apiClient.get<Role[]>(`/users/${id}/roles`)
  return response.data
}

export async function getUserGroups(id: number): Promise<Group[]> {
  const response = await apiClient.get<Group[]>(`/users/${id}/groups`)
  return response.data
}

export async function getUserPermissions(id: number): Promise<RepoPermission[]> {
  const response = await apiClient.get<RepoPermission[]>(`/users/${id}/permissions`)
  return response.data
}

export async function updateUserRole(id: number, data: UpdateUserRoleRequest): Promise<void> {
  await apiClient.put(`/users/${id}/role`, data)
}

export async function changeUserPassword(
  id: number,
  data: UpdateUserPasswordRequest,
): Promise<void> {
  await apiClient.put(`/users/${id}/password`, data)
}

export async function updateUserRoles(id: number, data: UpdateUserRolesRequest): Promise<void> {
  await apiClient.put(`/users/${id}/roles`, data)
}

export async function updateUserGroups(id: number, data: UpdateUserGroupsRequest): Promise<void> {
  await apiClient.put(`/users/${id}/groups`, data)
}
