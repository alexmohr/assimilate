// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type { GroupMembersResponse, GroupResponse } from '../types/generated'

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
