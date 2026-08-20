// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type { TagRow } from '../types/tag'

/** Tag namespace: which kind of entity a tag applies to. */
export type TagScope = 'repo' | 'host'

export async function listTags(scope: TagScope): Promise<TagRow[]> {
  const response = await apiClient.get<TagRow[]>('/tags', { params: { scope } })
  return response.data
}

export async function listEntityTags(entityPath: string): Promise<TagRow[]> {
  const response = await apiClient.get<TagRow[]>(`${entityPath}/tags`)
  return response.data
}

export async function setEntityTags(entityPath: string, tagIds: number[]): Promise<void> {
  await apiClient.put(`${entityPath}/tags`, { tag_ids: tagIds })
}

export async function createTag(name: string, color: string, scope: TagScope): Promise<TagRow> {
  const response = await apiClient.post<TagRow>('/tags', { name, color, scope })
  return response.data
}
