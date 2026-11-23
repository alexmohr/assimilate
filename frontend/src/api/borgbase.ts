// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  BorgbaseRepo,
  BorgbaseStatus,
  CreateBorgbaseRepoRequest,
  CreatedBorgbaseRepo,
} from '../types/generated'

/** Whether the server holds a BorgBase API token. */
export async function getBorgbaseStatus(): Promise<BorgbaseStatus> {
  const response = await apiClient.get<BorgbaseStatus>('/borgbase')
  return response.data
}

/** Stores the token after BorgBase has accepted it. The token is never returned. */
export async function setBorgbaseToken(token: string): Promise<BorgbaseStatus> {
  const response = await apiClient.put<BorgbaseStatus>('/borgbase/token', { token })
  return response.data
}

export async function deleteBorgbaseToken(): Promise<void> {
  await apiClient.delete('/borgbase/token')
}

/** The account's repositories with the quota and usage BorgBase reports. */
export async function listBorgbaseRepos(): Promise<BorgbaseRepo[]> {
  const response = await apiClient.get<BorgbaseRepo[]>('/borgbase/repos')
  return response.data
}

/** Creates a repository the server's SSH key may use. */
export async function createBorgbaseRepo(
  data: CreateBorgbaseRepoRequest,
): Promise<CreatedBorgbaseRepo> {
  const response = await apiClient.post<CreatedBorgbaseRepo>('/borgbase/repos', data)
  return response.data
}
