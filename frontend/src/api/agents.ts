// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import type { AgentHostnamePattern, AgentRow } from '../types/agent'
import type {
  AgentTagEntryResponse,
  CreateAgentResponse,
  DeployAgentResponse,
  MergeAgentResponse,
} from '../types/generated'
import type { Repo } from '../types/repo'
import type { ReportRow } from '../types/report'

export interface CreateAgentRequest {
  hostname: string
  display_name: string | null
}

export interface UpdateAgentRequest {
  hostname?: string
  display_name?: string | null
  default_backup_paths?: string[]
  default_exclude_patterns?: string[]
  default_pre_backup_commands?: string[]
  default_post_backup_commands?: string[]
  default_file_change_patterns_raw?: string
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
  target?: string
}

export async function listAgents(includeHidden?: boolean): Promise<AgentRow[]> {
  const params = includeHidden ? { include_hidden: true } : undefined
  const response = await apiClient.get<AgentRow[]>('/agents', { params })
  return response.data
}

export async function listAgentTags(): Promise<AgentTagEntryResponse[]> {
  const response = await apiClient.get<AgentTagEntryResponse[]>('/agent-tags')
  return response.data
}

export async function createAgent(data: CreateAgentRequest): Promise<CreateAgentResponse> {
  const response = await apiClient.post<CreateAgentResponse>('/agents', data)
  return response.data
}

export async function updateAgent(hostname: string, data: UpdateAgentRequest): Promise<AgentRow> {
  const response = await apiClient.put<AgentRow>(`/agents/${hostname}`, data)
  return response.data
}

export async function deleteAgent(hostname: string): Promise<void> {
  await apiClient.delete(`/agents/${hostname}`)
}

export async function hideAgent(hostname: string): Promise<void> {
  await apiClient.put(`/agents/${hostname}/hide`)
}

export async function unhideAgent(hostname: string): Promise<void> {
  await apiClient.put(`/agents/${hostname}/unhide`)
}

export async function regenerateAgentToken(hostname: string): Promise<CreateAgentResponse> {
  const response = await apiClient.post<CreateAgentResponse>(`/agents/${hostname}/regenerate-token`)
  return response.data
}

export async function restartAgent(hostname: string): Promise<void> {
  await apiClient.post(`/agents/${hostname}/restart`)
}

export async function deleteAgentArchives(hostname: string): Promise<void> {
  await apiClient.post(`/agents/${hostname}/delete-archives`)
}

export async function listAgentHostnamePatterns(hostname: string): Promise<AgentHostnamePattern[]> {
  const response = await apiClient.get<AgentHostnamePattern[]>(
    `/agents/${hostname}/hostname-patterns`,
  )
  return response.data
}

export async function createAgentHostnamePattern(
  hostname: string,
  pattern: string,
): Promise<AgentHostnamePattern> {
  const response = await apiClient.post<AgentHostnamePattern>(
    `/agents/${hostname}/hostname-patterns`,
    { pattern },
  )
  return response.data
}

export async function deleteAgentHostnamePattern(hostname: string, id: number): Promise<void> {
  await apiClient.delete(`/agents/${hostname}/hostname-patterns/${id}`)
}

export async function mergeAgent(
  targetHostname: string,
  sourceAgentId: number,
  createPattern?: string,
): Promise<MergeAgentResult> {
  const body: { create_pattern?: string } = {}
  if (createPattern) {
    body.create_pattern = createPattern
  }
  const response = await apiClient.post<MergeAgentResult>(
    `/agents/${targetHostname}/merge-from/${sourceAgentId}`,
    body,
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
): Promise<DeployAgentResult> {
  const response = await apiClient.post<DeployAgentResult>(`/agents/${hostname}/deploy`, data)
  return response.data
}

export async function deployAgentSshKey(data: DeploySshKeyRequest): Promise<DeploySshKeyResult> {
  const response = await apiClient.post<DeploySshKeyResult>('/ssh/deploy-key', data)
  return response.data
}

export async function listAgentRepos(hostname: string): Promise<Repo[]> {
  const response = await apiClient.get<Repo[]>(`/agents/${hostname}/repos`)
  return response.data
}

export async function listAgentReports(
  hostname: string,
  params?: ListAgentReportsParams,
): Promise<ReportRow[]> {
  const response = await apiClient.get<ReportRow[]>(`/agents/${hostname}/reports`, { params })
  return response.data
}
