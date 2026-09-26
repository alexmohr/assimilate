// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type { UpdateHostWakeRequest } from './agents'
import type { RepoHostKeyResponse, RepoHostResponse } from '../types/generated'

export type RepoHost = RepoHostResponse

export interface UpdateRepoHostRequest {
  ssh_host: string
  ssh_port: number
}

export async function listRepoHosts(): Promise<RepoHost[]> {
  const response = await apiClient.get<RepoHost[]>('/repo-hosts')
  return response.data
}

export async function getRepoHost(id: number): Promise<RepoHost> {
  const response = await apiClient.get<RepoHost>(`/repo-hosts/${id}`)
  return response.data
}

export async function updateRepoHost(id: number, data: UpdateRepoHostRequest): Promise<RepoHost> {
  const response = await apiClient.put<RepoHost>(`/repo-hosts/${id}`, data)
  return response.data
}

export async function deleteRepoHost(id: number): Promise<void> {
  await apiClient.delete(`/repo-hosts/${id}`)
}

export async function updateRepoHostPower(
  id: number,
  data: UpdateHostWakeRequest,
): Promise<RepoHost> {
  const response = await apiClient.put<RepoHost>(`/repo-hosts/${id}/power`, data)
  return response.data
}

export async function scanRepoHostKey(id: number): Promise<RepoHostKeyResponse> {
  const response = await apiClient.post<RepoHostKeyResponse>(`/repo-hosts/${id}/ssh-host-key/scan`)
  return response.data
}

export async function acceptRepoHostKey(id: number, sshHostKey: string): Promise<void> {
  await apiClient.post(`/repo-hosts/${id}/ssh-host-key`, { ssh_host_key: sshHostKey })
}
