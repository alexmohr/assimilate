// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import { domainParams } from '../utils/agent'
import type { AgentHostnamePattern, AgentRow } from '../types/agent'
import type {
  AgentTagEntryResponse,
  CreateAgentResponse,
  DeleteFailedReportsResponse,
  DeployAgentResponse,
  FailedReportCountResponse,
  HookCommand,
  MergeAgentResponse,
  ReportListResponse,
} from '../types/generated'
import type { Repo } from '../types/repo'

export interface CreateAgentRequest {
  hostname: string
  display_name: string | null
  domain?: string | null
}

export interface UpdateAgentRequest {
  hostname?: string
  display_name?: string | null
  domain?: string | null
  default_backup_paths?: string[]
  default_exclude_patterns?: string[]
  default_pre_backup_commands?: HookCommand[]
  default_post_backup_commands?: HookCommand[]
  default_file_change_patterns_raw?: string
}

/** A host's wake-before-backup settings, shared by agent and repo hosts. */
export interface UpdateHostWakeRequest {
  wake_enabled: boolean
  wake_mac_address: string | null
  wake_broadcast_address: string | null
  wake_timeout_seconds: number
  shutdown_after_backup: boolean
}

export interface UpdateAgentPowerRequest {
  wake: UpdateHostWakeRequest
  start_agent_enabled: boolean
  stop_agent_after_backup: boolean
  ssh_host: string | null
  ssh_port: number
  agent_service_name: string
}

export type MergeAgentResult = MergeAgentResponse

export interface ServiceUnitPreviewRequest {
  ssh_host: string
  ssh_user: string
  ssh_port: number
  ssh_password?: string
}

export interface ServiceUnitPreviewResponse {
  content: string | null
}

export interface DeployAgentRequest {
  ssh_host: string
  ssh_user: string
  ssh_port: number
  ssh_password?: string
  server_url: string
  install_path?: string
  systemd_service_content?: string
  force?: boolean
}

export type DeployAgentResult = DeployAgentResponse

export interface DeploySshKeyRequest {
  ssh_host: string
  ssh_user: string
  ssh_port: number
  password: string
  use_sftp: boolean
}

export interface DeploySshKeyResult {
  success: boolean
  already_deployed: boolean
  error?: string
}

export interface ListAgentReportsParams {
  limit?: number
  offset?: number
  target?: string
}

export async function listAgents(includeHidden?: boolean): Promise<AgentRow[]> {
  const params = includeHidden ? { include_hidden: true } : undefined
  const response = await apiClient.get<AgentRow[]>('/agents', { params })
  return response.data
}

export async function listAgentTags(options?: {
  timeout?: number
}): Promise<AgentTagEntryResponse[]> {
  const response = await apiClient.get<AgentTagEntryResponse[]>('/agent-tags', {
    timeout: options?.timeout,
  })
  return response.data
}

export async function createAgent(data: CreateAgentRequest): Promise<CreateAgentResponse> {
  const response = await apiClient.post<CreateAgentResponse>('/agents', data)
  return response.data
}

/**
 * @param domain Disambiguates `hostname` when it is shared by more than one
 *   agent. This identifies which existing agent to update, distinct from
 *   `data.domain`, which is the (possibly new) domain value to save.
 */
export async function updateAgent(
  hostname: string,
  data: UpdateAgentRequest,
  domain?: string | null,
): Promise<AgentRow> {
  const response = await apiClient.put<AgentRow>(`/agents/${hostname}`, data, {
    params: domainParams(domain),
  })
  return response.data
}

export async function updateAgentPower(
  hostname: string,
  data: UpdateAgentPowerRequest,
  domain?: string | null,
): Promise<AgentRow> {
  const response = await apiClient.put<AgentRow>(`/agents/${hostname}/power`, data, {
    params: domainParams(domain),
  })
  return response.data
}

export async function deleteAgent(hostname: string, domain?: string | null): Promise<void> {
  await apiClient.delete(`/agents/${hostname}`, { params: domainParams(domain) })
}

export async function hideAgent(hostname: string, domain?: string | null): Promise<void> {
  await apiClient.put(`/agents/${hostname}/hide`, {}, { params: domainParams(domain) })
}

export async function unhideAgent(hostname: string, domain?: string | null): Promise<void> {
  await apiClient.put(`/agents/${hostname}/unhide`, {}, { params: domainParams(domain) })
}

export async function regenerateAgentToken(
  hostname: string,
  domain?: string | null,
): Promise<CreateAgentResponse> {
  const response = await apiClient.post<CreateAgentResponse>(
    `/agents/${hostname}/regenerate-token`,
    {},
    { params: domainParams(domain) },
  )
  return response.data
}

export async function restartAgent(hostname: string, domain?: string | null): Promise<void> {
  await apiClient.post(`/agents/${hostname}/restart`, {}, { params: domainParams(domain) })
}

export async function deleteAgentArchives(hostname: string, domain?: string | null): Promise<void> {
  await apiClient.post(`/agents/${hostname}/delete-archives`, {}, { params: domainParams(domain) })
}

export async function cancelAgentBackup(
  hostname: string,
  repoId: number,
  domain?: string | null,
): Promise<void> {
  await apiClient.post(
    `/agents/${hostname}/repos/${repoId}/cancel-backup`,
    {},
    { params: domainParams(domain) },
  )
}

export async function deleteFailedReports(
  hostname: string,
  domain?: string | null,
): Promise<DeleteFailedReportsResponse> {
  const response = await apiClient.delete<DeleteFailedReportsResponse>(
    `/agents/${hostname}/reports/failed`,
    { params: domainParams(domain) },
  )
  return response.data
}

/**
 * Unbounded by the report list's own pagination window - the true count a
 * "clean up failed backups" confirmation is about to delete.
 */
export async function countFailedReports(
  hostname: string,
  domain?: string | null,
): Promise<number> {
  const response = await apiClient.get<FailedReportCountResponse>(
    `/agents/${hostname}/reports/failed/count`,
    { params: domainParams(domain) },
  )
  return response.data.count
}

export async function listAgentHostnamePatterns(
  hostname: string,
  domain?: string | null,
): Promise<AgentHostnamePattern[]> {
  const response = await apiClient.get<AgentHostnamePattern[]>(
    `/agents/${hostname}/hostname-patterns`,
    { params: domainParams(domain) },
  )
  return response.data
}

export async function createAgentHostnamePattern(
  hostname: string,
  pattern: string,
  domain?: string | null,
): Promise<AgentHostnamePattern> {
  const response = await apiClient.post<AgentHostnamePattern>(
    `/agents/${hostname}/hostname-patterns`,
    { pattern },
    { params: domainParams(domain) },
  )
  return response.data
}

export async function deleteAgentHostnamePattern(
  hostname: string,
  id: number,
  domain?: string | null,
): Promise<void> {
  await apiClient.delete(`/agents/${hostname}/hostname-patterns/${id}`, {
    params: domainParams(domain),
  })
}

export async function mergeAgent(
  targetHostname: string,
  sourceAgentId: number,
  createPattern?: string,
  targetDomain?: string | null,
): Promise<MergeAgentResult> {
  const body: { create_pattern?: string } = {}
  if (createPattern) {
    body.create_pattern = createPattern
  }
  const response = await apiClient.post<MergeAgentResult>(
    `/agents/${targetHostname}/merge-from/${sourceAgentId}`,
    body,
    { params: domainParams(targetDomain) },
  )
  return response.data
}

export async function previewAgentServiceUnit(
  hostname: string,
  data: ServiceUnitPreviewRequest,
): Promise<ServiceUnitPreviewResponse> {
  const response = await apiClient.post<ServiceUnitPreviewResponse>(
    `/agents/${hostname}/service-unit`,
    data,
  )
  return response.data
}

export async function deployAgent(
  hostname: string,
  data: DeployAgentRequest,
  domain?: string | null,
): Promise<DeployAgentResult> {
  const response = await apiClient.post<DeployAgentResult>(`/agents/${hostname}/deploy`, data, {
    params: domainParams(domain),
  })
  return response.data
}

export async function deployAgentSshKey(data: DeploySshKeyRequest): Promise<DeploySshKeyResult> {
  const response = await apiClient.post<DeploySshKeyResult>('/ssh/deploy-key', data)
  return response.data
}

export async function listAgentRepos(hostname: string, domain?: string | null): Promise<Repo[]> {
  const response = await apiClient.get<Repo[]>(`/agents/${hostname}/repos`, {
    params: domainParams(domain),
  })
  return response.data
}

export async function listAgentReports(
  hostname: string,
  params?: ListAgentReportsParams,
  domain?: string | null,
): Promise<ReportListResponse> {
  const response = await apiClient.get<ReportListResponse>(`/agents/${hostname}/reports`, {
    params: { ...params, ...domainParams(domain) },
  })
  return response.data
}
