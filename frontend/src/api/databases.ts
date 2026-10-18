// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import { domainParams } from '../utils/agent'
import type {
  AgentDatabasesResponse,
  DatabaseRuntime,
  DatabaseRuntimes,
  DatabaseSelectionMode,
} from '../types/generated'

/** New database dump settings for a host. */
export interface UpdateAgentDatabaseBackupRequest {
  enabled: boolean
  dump_dir: string
  /** Where the host looks for databases. Empty paths mean the agent's defaults. */
  runtimes: DatabaseRuntimes
  /** Omitted keeps whichever mode the host already had. */
  selection?: DatabaseSelectionMode
}

/** One database source, as the server tells sources apart. */
export interface DatabaseSourceRef {
  runtime: DatabaseRuntime
  namespace: string
  name: string
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
  source: DatabaseSourceRef,
  included: boolean | null,
  domain?: string | null,
): Promise<AgentDatabasesResponse> {
  const response = await apiClient.put<AgentDatabasesResponse>(
    `${sourcePath(hostname)}/sources`,
    {
      runtime: source.runtime,
      namespace: source.namespace,
      name: source.name,
      included,
    },
    { params: domainParams(domain) },
  )
  return response.data
}

/**
 * Asks the agent which database workloads it runs. Runtime settings try
 * those instead of the saved ones, so they can be tested before they are
 * saved.
 */
export async function scanAgentDatabases(
  hostname: string,
  runtimes: DatabaseRuntimes | null,
  domain?: string | null,
): Promise<AgentDatabasesResponse> {
  const response = await apiClient.post<AgentDatabasesResponse>(
    `${sourcePath(hostname)}/scan`,
    { runtimes },
    { params: domainParams(domain) },
  )
  return response.data
}
