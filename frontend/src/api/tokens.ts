// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  ApiTokenResponse,
  CreateApiTokenResponse,
  ListApiTokensResponse,
} from '../types/generated'

export type ApiToken = ApiTokenResponse

export async function listTokens(): Promise<ApiToken[]> {
  const response = await apiClient.get<ListApiTokensResponse>('/tokens')
  return response.data.tokens
}

export async function createToken(name: string): Promise<CreateApiTokenResponse> {
  const response = await apiClient.post<CreateApiTokenResponse>('/tokens', { name })
  return response.data
}

export async function deleteToken(id: number): Promise<void> {
  await apiClient.delete(`/tokens/${id}`)
}
