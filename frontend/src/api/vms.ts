// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import { domainParams } from '../utils/agent'
import type { AgentVmSnapshotResponse } from '../types/generated'

/** New staging settings for a host. */
export interface UpdateAgentVmSnapshotRequest {
  enabled: boolean
  staging_dir: string
  full_interval: number
  timeout_seconds: number
  default_limit_bytes: number
}

/** New settings for one domain of a host. */
export interface UpdateAgentVmRequest {
  included: boolean
  /** Null inherits the host's default limit. */
  limit_bytes: number | null
}

export async function getAgentVms(
  hostname: string,
  domain?: string | null,
): Promise<AgentVmSnapshotResponse> {
  const response = await apiClient.get<AgentVmSnapshotResponse>(`/agents/${hostname}/vms`, {
    params: domainParams(domain),
  })
  return response.data
}

export async function updateAgentVmSnapshot(
  hostname: string,
  data: UpdateAgentVmSnapshotRequest,
  domain?: string | null,
): Promise<AgentVmSnapshotResponse> {
  const response = await apiClient.put<AgentVmSnapshotResponse>(
    `/agents/${hostname}/vm-snapshot`,
    data,
    { params: domainParams(domain) },
  )
  return response.data
}

export async function updateAgentVm(
  hostname: string,
  name: string,
  data: UpdateAgentVmRequest,
  domain?: string | null,
): Promise<AgentVmSnapshotResponse> {
  const response = await apiClient.put<AgentVmSnapshotResponse>(
    `/agents/${hostname}/vms/${encodeURIComponent(name)}`,
    data,
    { params: domainParams(domain) },
  )
  return response.data
}

/** Asks the host's agent which domains it has and waits for the answer. */
export async function scanAgentVms(
  hostname: string,
  domain?: string | null,
): Promise<AgentVmSnapshotResponse> {
  const response = await apiClient.post<AgentVmSnapshotResponse>(
    `/agents/${hostname}/vms/scan`,
    {},
    { params: domainParams(domain) },
  )
  return response.data
}
