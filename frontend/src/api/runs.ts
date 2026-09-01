// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type { RunEventResponse } from '../types/generated'

/**
 * Fetches one target pairing's slice of a run's power-management timeline.
 * `runId` alone spans every target of a multi-target schedule, so
 * `agentId`/`repoId` narrow to the pairing this report is for.
 */
export async function getRunEvents(
  runId: string,
  agentId: number,
  repoId: number,
): Promise<RunEventResponse[]> {
  const response = await apiClient.get<RunEventResponse[]>(`/runs/${runId}/events`, {
    params: { agent_id: agentId, repo_id: repoId },
  })
  return response.data
}
