// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import { domainParams } from '../utils/agent'
import type {
  AgentVmSnapshotResponse,
  VmBuildAction,
  VmBuildOutcome,
  VmSelectionMode,
} from '../types/generated'

/** New staging settings for a host. */
export interface UpdateAgentVmSnapshotRequest {
  enabled: boolean
  staging_dir: string
  full_interval: number
  timeout_seconds: number
  default_limit_bytes: number
  /** Omitted keeps whichever mode the host already had. */
  selection?: VmSelectionMode
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

/** What to build out of a restored domain directory, and what to do with it. */
export interface BuildAgentVmRequest {
  source_dir: string
  name: string
  image_dir: string
  action: VmBuildAction
}

/**
 * Stage two of a virtual-machine restore: merge the chain, place the images
 * and define the domain. Reads whatever directory it is pointed at, so it also
 * works on files restored earlier.
 */
export async function buildAgentVm(
  hostname: string,
  data: BuildAgentVmRequest,
  domain?: string | null,
): Promise<VmBuildOutcome> {
  const response = await apiClient.post<VmBuildOutcome>(`/agents/${hostname}/vms/build`, data, {
    params: domainParams(domain),
  })
  return response.data
}
