// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type { RunEventResponse } from '../types/generated'

export async function getRunEvents(runId: string): Promise<RunEventResponse[]> {
  const response = await apiClient.get<RunEventResponse[]>(`/runs/${runId}/events`)
  return response.data
}
