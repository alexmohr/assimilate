// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'

export interface Group {
  id: number
  name: string
  description: string | null
  created_at: string
}

export interface GroupMember {
  user_id: number
}

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

export async function listGroupMembers(id: number): Promise<GroupMember[]> {
  const response = await apiClient.get<GroupMember[]>(`/groups/${id}/members`)
  return response.data
}

export async function updateGroupMembers(
  id: number,
  data: UpdateGroupMembersRequest,
): Promise<void> {
  await apiClient.put(`/groups/${id}/members`, data)
}
