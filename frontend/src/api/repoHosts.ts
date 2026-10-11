// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type {
  RepoHostDependenciesBody,
  RepoHostKeyResponse,
  RepoHostResponse,
} from '../types/generated'

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

/** The dependencies every schedule writing to a repository on this host needs. */
export async function getRepoHostDependencies(id: number): Promise<number[]> {
  const response = await apiClient.get<RepoHostDependenciesBody>(`/repo-hosts/${id}/dependencies`)
  return response.data.dependency_host_ids
}

export async function updateRepoHostDependencies(
  id: number,
  dependencyHostIds: number[],
): Promise<number[]> {
  const response = await apiClient.put<RepoHostDependenciesBody>(`/repo-hosts/${id}/dependencies`, {
    dependency_host_ids: dependencyHostIds,
  })
  return response.data.dependency_host_ids
}

export async function scanRepoHostKey(id: number): Promise<RepoHostKeyResponse> {
  const response = await apiClient.post<RepoHostKeyResponse>(`/repo-hosts/${id}/ssh-host-key/scan`)
  return response.data
}

export async function acceptRepoHostKey(id: number, sshHostKey: string): Promise<void> {
  await apiClient.post(`/repo-hosts/${id}/ssh-host-key`, { ssh_host_key: sshHostKey })
}
