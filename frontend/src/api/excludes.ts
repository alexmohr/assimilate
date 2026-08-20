// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type { GlobalExcludesResponse } from '../types/generated'

export interface SetGlobalExcludesRequest {
  raw_text: string
}

export async function getExcludes(): Promise<GlobalExcludesResponse> {
  const response = await apiClient.get<GlobalExcludesResponse>('/excludes')
  return response.data
}

export async function setExcludes(data: SetGlobalExcludesRequest): Promise<GlobalExcludesResponse> {
  const response = await apiClient.put<GlobalExcludesResponse>('/excludes', data)
  return response.data
}
