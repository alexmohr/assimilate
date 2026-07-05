// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type { QuotaAction, ServerQuotaResponse } from '../types/generated'

export interface UpsertServerQuotaRequest {
  warn_bytes: number
  critical_bytes: number
  warn_action: QuotaAction
  critical_action: QuotaAction
  enabled: boolean
}

export async function listServerQuotas(): Promise<ServerQuotaResponse[]> {
  const response = await apiClient.get<ServerQuotaResponse[]>('/server-quotas')
  return response.data
}

export async function upsertServerQuota(
  sshHost: string,
  data: UpsertServerQuotaRequest,
): Promise<ServerQuotaResponse> {
  const response = await apiClient.put<ServerQuotaResponse>(
    `/server-quotas/${encodeURIComponent(sshHost)}`,
    data,
  )
  return response.data
}

export async function deleteServerQuota(sshHost: string): Promise<void> {
  await apiClient.delete(`/server-quotas/${encodeURIComponent(sshHost)}`)
}
