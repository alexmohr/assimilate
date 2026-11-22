// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  CreateEnrollmentTokenRequest,
  CreateEnrollmentTokenResponse,
  ListEnrollmentTokensResponse,
} from '../types/generated'

export async function createEnrollmentToken(
  request: CreateEnrollmentTokenRequest,
): Promise<CreateEnrollmentTokenResponse> {
  const response = await apiClient.post<CreateEnrollmentTokenResponse>(
    '/enrollment-tokens',
    request,
  )
  return response.data
}

export async function listEnrollmentTokens(): Promise<ListEnrollmentTokensResponse> {
  const response = await apiClient.get<ListEnrollmentTokensResponse>('/enrollment-tokens')
  return response.data
}

export async function revokeEnrollmentToken(id: number): Promise<void> {
  await apiClient.delete(`/enrollment-tokens/${id}`)
}
