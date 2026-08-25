// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type { Repo, RepoWithStats } from '../types/repo'
import type {
  BreakLockResponse,
  ConfirmRelocationResponse,
  ExecBorgResponse,
  PassphraseResponse,
  QuotaAction,
  RepoHostKeyResponse,
  RepoQuotaResponse,
  RepoTagEntryResponse,
  RescanResponse,
} from '../types/generated'

export async function listRepos(): Promise<Repo[]> {
  const response = await apiClient.get<Repo[]>('/repos')
  return response.data
}

export async function listRepoStats(): Promise<RepoWithStats[]> {
  const response = await apiClient.get<RepoWithStats[]>('/repos/stats')
  return response.data
}

export async function listRepoTags(): Promise<RepoTagEntryResponse[]> {
  const response = await apiClient.get<RepoTagEntryResponse[]>('/repo-tags')
  return response.data
}

export async function getRepo(id: number): Promise<RepoWithStats> {
  const response = await apiClient.get<RepoWithStats>(`/repos/${id}`)
  return response.data
}

export interface CreateRepoRequest {
  name: string
  repo_path: string
  ssh_user: string
  ssh_host: string
  ssh_port: number
  passphrase: string
  compression: string
}

export async function createRepo(data: CreateRepoRequest): Promise<Repo> {
  const response = await apiClient.post<Repo>('/repos', data)
  return response.data
}

export interface InitRepoRequest {
  name: string
  repo_path: string
  ssh_user: string
  ssh_host: string
  ssh_port: number
  passphrase: string
  encryption: string
  compression: string
}

export async function initRepo(data: InitRepoRequest): Promise<void> {
  await apiClient.post('/repos/init', data)
}

export interface UpdateRepoRequest {
  name: string
  repo_path: string
  ssh_user: string
  ssh_host: string
  ssh_port: number
  compression: string
  encryption: string
  enabled: boolean
  sync_schedule: string | null
}

export async function updateRepo(id: number, data: UpdateRepoRequest): Promise<void> {
  await apiClient.put(`/repos/${id}`, data)
}

export async function deleteRepo(id: number): Promise<void> {
  await apiClient.delete(`/repos/${id}`)
}

export async function destroyRepo(id: number): Promise<void> {
  await apiClient.post(`/repos/${id}/destroy`)
}

export async function syncRepo(id: number): Promise<void> {
  await apiClient.post(`/repos/${id}/sync?build_index=true`)
}

export async function resetImportRepo(id: number): Promise<void> {
  await apiClient.post(`/repos/${id}/reset-import`)
}

export async function resetAndSyncRepo(id: number): Promise<void> {
  await apiClient.post(`/repos/${id}/reset-and-sync?build_index=true`)
}

export async function getRepoPassphrase(id: number): Promise<PassphraseResponse> {
  const response = await apiClient.get<PassphraseResponse>(`/repos/${id}/passphrase`)
  return response.data
}

export async function scanRepoSshHostKey(id: number): Promise<RepoHostKeyResponse> {
  const response = await apiClient.post<RepoHostKeyResponse>(`/repos/${id}/ssh-host-key/scan`)
  return response.data
}

export async function acceptRepoSshHostKey(id: number, sshHostKey: string): Promise<void> {
  await apiClient.post(`/repos/${id}/ssh-host-key`, { ssh_host_key: sshHostKey })
}

export async function confirmRepoRelocation(id: number): Promise<ConfirmRelocationResponse> {
  const response = await apiClient.post<ConfirmRelocationResponse>(
    `/repos/${id}/confirm-relocation`,
  )
  return response.data
}

export async function breakRepoLock(id: number): Promise<BreakLockResponse> {
  const response = await apiClient.post<BreakLockResponse>(`/repos/${id}/break-lock`)
  return response.data
}

export async function execRepoCommand(id: number, args: string[]): Promise<ExecBorgResponse> {
  const response = await apiClient.post<ExecBorgResponse>(`/repos/${id}/exec`, { args })
  return response.data
}

export async function rescanRepo(id: number): Promise<RescanResponse> {
  const response = await apiClient.post<RescanResponse>(`/repos/${id}/rescan`)
  return response.data
}

export interface TestRepoConnectionRequest {
  ssh_host: string
  ssh_user: string
  ssh_port: number
}

export interface TestRepoConnectionResponse {
  ssh_ok: boolean
  borg_installed: boolean
  borg_version?: string
  error?: string
}

export async function testRepoConnection(
  data: TestRepoConnectionRequest,
): Promise<TestRepoConnectionResponse> {
  const response = await apiClient.post<TestRepoConnectionResponse>('/ssh/test-connection', data)
  return response.data
}

export type QuotaData = RepoQuotaResponse

export async function getRepoQuota(id: number): Promise<QuotaData> {
  const response = await apiClient.get<QuotaData>(`/repos/${id}/quota`)
  return response.data
}

export interface UpdateRepoQuotaRequest {
  warn_bytes: number
  critical_bytes: number
  warn_action: QuotaAction
  critical_action: QuotaAction
  enabled: boolean
}

export async function updateRepoQuota(id: number, data: UpdateRepoQuotaRequest): Promise<void> {
  await apiClient.put(`/repos/${id}/quota`, data)
}

export interface UpdateRepoPermissionRequest {
  can_view: boolean
  can_backup: boolean
  can_modify_schedules: boolean
  can_extract: boolean
  can_delete: boolean
}

export async function updateRepoPermission(
  repoId: number,
  userId: number,
  data: UpdateRepoPermissionRequest,
): Promise<void> {
  await apiClient.put(`/repos/${repoId}/permissions/${userId}`, data)
}
