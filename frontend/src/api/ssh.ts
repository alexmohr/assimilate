// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'

/** A generic SSH connection target, used by the repo-creation folder browser. */
export interface SshTargetRequest {
  ssh_host: string
  ssh_user: string
  ssh_port: number
}

export interface SshDirEntry {
  name: string
  is_dir: boolean
}

export interface ListSshDirectoryRequest extends SshTargetRequest {
  path: string
}

export interface ListSshDirectoryResponse {
  path: string
  entries: SshDirEntry[]
  error?: string
}

export async function listSshDirectory(
  data: ListSshDirectoryRequest,
): Promise<ListSshDirectoryResponse> {
  const response = await apiClient.post<ListSshDirectoryResponse>('/ssh/list-dir', data)
  return response.data
}

export interface CreateSshDirectoryRequest extends SshTargetRequest {
  path: string
}

export async function createSshDirectory(data: CreateSshDirectoryRequest): Promise<void> {
  await apiClient.post('/ssh/mkdir', data)
}
