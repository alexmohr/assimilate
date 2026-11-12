// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  AppendOnlyStatusResponse,
  ReapplyAppendOnlyRequest,
  SetAppendOnlyRequest,
  UpdateMaintenanceRequest,
} from '../types/generated'

function base(repoId: number): string {
  return `/repos/${repoId}/append-only`
}

export async function getAppendOnlyStatus(repoId: number): Promise<AppendOnlyStatusResponse> {
  const response = await apiClient.get<AppendOnlyStatusResponse>(base(repoId))
  return response.data
}

/**
 * Always answers 200: a failed setup comes back in `setup_failure`, with
 * `needs_password` set when the host's SSH password is needed once.
 */
export async function setAppendOnly(
  repoId: number,
  data: SetAppendOnlyRequest,
): Promise<AppendOnlyStatusResponse> {
  const response = await apiClient.put<AppendOnlyStatusResponse>(base(repoId), data)
  return response.data
}

export async function checkAppendOnly(repoId: number): Promise<AppendOnlyStatusResponse> {
  const response = await apiClient.post<AppendOnlyStatusResponse>(`${base(repoId)}/check`)
  return response.data
}

export async function reapplyAppendOnly(
  repoId: number,
  data: ReapplyAppendOnlyRequest,
): Promise<AppendOnlyStatusResponse> {
  const response = await apiClient.post<AppendOnlyStatusResponse>(`${base(repoId)}/reapply`, data)
  return response.data
}

export async function updateMaintenance(
  repoId: number,
  data: UpdateMaintenanceRequest,
): Promise<AppendOnlyStatusResponse> {
  const response = await apiClient.put<AppendOnlyStatusResponse>(
    `${base(repoId)}/maintenance`,
    data,
  )
  return response.data
}

/** Starts a run in the background (202, no body). */
export async function runMaintenance(repoId: number): Promise<void> {
  await apiClient.post(`${base(repoId)}/maintenance/run`)
}

export async function resumeMaintenance(repoId: number): Promise<AppendOnlyStatusResponse> {
  const response = await apiClient.post<AppendOnlyStatusResponse>(
    `${base(repoId)}/maintenance/resume`,
  )
  return response.data
}

/** Undoes a queued deletion (the Keep button). */
export async function keepArchive(repoId: number, archiveName: string): Promise<void> {
  await apiClient.delete(`/repos/${repoId}/archives/${encodeURIComponent(archiveName)}/deletion`)
}
