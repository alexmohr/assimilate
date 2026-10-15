// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import { domainParams } from '../utils/agent'
import type { AgentDatabasesResponse, DatabaseSelectionMode } from '../types/generated'

/** New database dump settings for a host. */
export interface UpdateAgentDatabaseBackupRequest {
  enabled: boolean
  dump_dir: string
  /** Empty or null uses the agent's default kubeconfig. */
  kubeconfig_path: string | null
  /** Omitted keeps whichever mode the host already had. */
  selection?: DatabaseSelectionMode
}

function sourcePath(hostname: string): string {
  return `/agents/${hostname}/databases`
}

export async function getAgentDatabases(
  hostname: string,
  domain?: string | null,
): Promise<AgentDatabasesResponse> {
  const response = await apiClient.get<AgentDatabasesResponse>(sourcePath(hostname), {
    params: domainParams(domain),
  })
  return response.data
}

export async function updateAgentDatabaseBackup(
  hostname: string,
  data: UpdateAgentDatabaseBackupRequest,
  domain?: string | null,
): Promise<AgentDatabasesResponse> {
  const response = await apiClient.put<AgentDatabasesResponse>(
    `/agents/${hostname}/database-backup`,
    data,
    { params: domainParams(domain) },
  )
  return response.data
}

/** Chooses whether backups dump one source; null hands it back to the mode. */
export async function updateAgentDatabaseSource(
  hostname: string,
  namespace: string,
  name: string,
  included: boolean | null,
  domain?: string | null,
): Promise<AgentDatabasesResponse> {
  const response = await apiClient.put<AgentDatabasesResponse>(
    `${sourcePath(hostname)}/${encodeURIComponent(namespace)}/${encodeURIComponent(name)}`,
    { included },
    { params: domainParams(domain) },
  )
  return response.data
}

/**
 * Asks the agent which database workloads its cluster runs. A kubeconfig
 * path tries that file instead of the saved one, so it can be tested before
 * it is saved.
 */
export async function scanAgentDatabases(
  hostname: string,
  kubeconfigPath: string | null,
  domain?: string | null,
): Promise<AgentDatabasesResponse> {
  const response = await apiClient.post<AgentDatabasesResponse>(
    `${sourcePath(hostname)}/scan`,
    { kubeconfig_path: kubeconfigPath },
    { params: domainParams(domain) },
  )
  return response.data
}
