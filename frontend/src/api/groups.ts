// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  GroupMembersResponse,
  GroupRepoPermissionResponse,
  GroupResponse,
  RepoPermission,
} from '../types/generated'

export type Group = GroupResponse

export interface CreateGroupRequest {
  name: string
  description: string | null
}

export type UpdateGroupRequest = CreateGroupRequest

export interface UpdateGroupMembersRequest {
  user_ids: number[]
}

export async function listGroups(): Promise<Group[]> {
  const response = await apiClient.get<Group[]>('/groups')
  return response.data
}

export async function createGroup(data: CreateGroupRequest): Promise<void> {
  await apiClient.post('/groups', data)
}

export async function updateGroup(id: number, data: UpdateGroupRequest): Promise<void> {
  await apiClient.put(`/groups/${id}`, data)
}

export async function deleteGroup(id: number): Promise<void> {
  await apiClient.delete(`/groups/${id}`)
}

export async function listGroupMembers(id: number): Promise<number[]> {
  const response = await apiClient.get<GroupMembersResponse>(`/groups/${id}/members`)
  return response.data.user_ids
}

export async function updateGroupMembers(
  id: number,
  data: UpdateGroupMembersRequest,
): Promise<void> {
  await apiClient.put(`/groups/${id}/members`, data)
}

/** Every repository grant of a group. */
export async function listGroupPermissions(id: number): Promise<GroupRepoPermissionResponse[]> {
  const response = await apiClient.get<GroupRepoPermissionResponse[]>(`/groups/${id}/permissions`)
  return response.data
}

/** Sets what a group's members may do on a repository; an empty list revokes the grant. */
export async function setGroupPermission(
  groupId: number,
  repoId: number,
  permissions: RepoPermission[],
): Promise<GroupRepoPermissionResponse> {
  const response = await apiClient.put<GroupRepoPermissionResponse>(
    `/groups/${groupId}/permissions/${repoId}`,
    { permissions },
  )
  return response.data
}
