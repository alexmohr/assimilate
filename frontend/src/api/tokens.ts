// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  ApiTokenResponse,
  ApiTokenScope,
  CreateApiTokenResponse,
  ListApiTokensResponse,
} from '../types/generated'

export type ApiToken = ApiTokenResponse

export async function listTokens(): Promise<ApiToken[]> {
  const response = await apiClient.get<ListApiTokensResponse>('/tokens')
  return response.data.tokens
}

/**
 * Creates a token. A full token is the server's default, so its request
 * carries only the name, exactly as it did before tokens had a scope.
 */
export async function createToken(
  name: string,
  scope: ApiTokenScope = 'full',
): Promise<CreateApiTokenResponse> {
  const body = scope === 'full' ? { name } : { name, scope }
  const response = await apiClient.post<CreateApiTokenResponse>('/tokens', body)
  return response.data
}

export async function deleteToken(id: number): Promise<void> {
  await apiClient.delete(`/tokens/${id}`)
}
