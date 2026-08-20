// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type { AuditLogResponse } from '../types/generated'

export interface AuditLogQuery {
  page: number
  per_page: number
  action?: string
  user_id?: string
  from?: string
  to?: string
}

export async function getAuditLog(query: AuditLogQuery): Promise<AuditLogResponse> {
  const params: Record<string, string | number> = {
    page: query.page,
    per_page: query.per_page,
  }
  if (query.action) params.action = query.action
  if (query.user_id) params.user_id = query.user_id
  if (query.from) params.from = query.from
  if (query.to) params.to = query.to

  const response = await apiClient.get<AuditLogResponse>('/audit-log', { params })
  return response.data
}
