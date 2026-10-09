// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type { RestoreRun } from '../types/generated'

/** Recent restores onto agents, newest first. */
export async function listRestoreRuns(limit?: number): Promise<RestoreRun[]> {
  const response = await apiClient.get<RestoreRun[]>('/restores', {
    params: limit === undefined ? {} : { limit },
  })
  return response.data
}

export async function getRestoreRun(id: string): Promise<RestoreRun> {
  const response = await apiClient.get<RestoreRun>(`/restores/${encodeURIComponent(id)}`)
  return response.data
}

/** Cancels a restore still waiting for its agent to connect. */
export async function cancelRestoreRun(id: string): Promise<RestoreRun> {
  const response = await apiClient.post<RestoreRun>(`/restores/${encodeURIComponent(id)}/cancel`)
  return response.data
}
