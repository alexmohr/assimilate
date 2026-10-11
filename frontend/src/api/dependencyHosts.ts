// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import { domainParams } from '../utils/agent'
import type {
  AgentDependenciesBody,
  DependencyHostResponse,
  DependencyTestResponse,
  DependencyUsageResponse,
  ScheduleDependenciesResponse,
  ScheduleDependencyInput,
} from '../types/generated'

export type DependencyHost = DependencyHostResponse

/** A new dependency: where it is, which port says it is up, and what it is for. */
export interface CreateDependencyHostRequest {
  name: string
  address: string
  port: number
  description: string
  /** The repository host it is the same machine as, to share its wake settings. */
  repo_host_id: number | null
}

export interface UpdateDependencyHostRequest {
  name: string
  address: string
  port: number
  description: string
}

export async function listDependencyHosts(): Promise<DependencyHost[]> {
  const response = await apiClient.get<DependencyHost[]>('/dependency-hosts')
  return response.data
}

export async function getDependencyHost(id: number): Promise<DependencyHost> {
  const response = await apiClient.get<DependencyHost>(`/dependency-hosts/${id}`)
  return response.data
}

export async function createDependencyHost(
  data: CreateDependencyHostRequest,
): Promise<DependencyHost> {
  const response = await apiClient.post<DependencyHost>('/dependency-hosts', data)
  return response.data
}

export async function updateDependencyHost(
  id: number,
  data: UpdateDependencyHostRequest,
): Promise<DependencyHost> {
  const response = await apiClient.put<DependencyHost>(`/dependency-hosts/${id}`, data)
  return response.data
}

export async function deleteDependencyHost(id: number): Promise<void> {
  await apiClient.delete(`/dependency-hosts/${id}`)
}

/**
 * Puts the dependency on the machine of a repository host, sharing its wake
 * and availability settings, or with `null` on a machine of its own.
 */
export async function updateDependencyHostMachine(
  id: number,
  repoHostId: number | null,
): Promise<DependencyHost> {
  const response = await apiClient.put<DependencyHost>(`/dependency-hosts/${id}/machine`, {
    repo_host_id: repoHostId,
  })
  return response.data
}

/** Checks a saved dependency without waking it or starting anything. */
export async function testDependencyHost(id: number): Promise<DependencyTestResponse> {
  const response = await apiClient.post<DependencyTestResponse>(`/dependency-hosts/${id}/test`)
  return response.data
}

/** Checks an address and port before they are saved. */
export async function testDependencyAddress(
  address: string,
  port: number,
): Promise<DependencyTestResponse> {
  const response = await apiClient.post<DependencyTestResponse>('/dependency-hosts/test', {
    address,
    port,
  })
  return response.data
}

export async function listDependencyHostUsage(id: number): Promise<DependencyUsageResponse[]> {
  const response = await apiClient.get<DependencyUsageResponse[]>(`/dependency-hosts/${id}/usage`)
  return response.data
}

export async function getScheduleDependencies(
  scheduleId: number,
): Promise<ScheduleDependenciesResponse> {
  const response = await apiClient.get<ScheduleDependenciesResponse>(
    `/schedules/${scheduleId}/dependencies`,
  )
  return response.data
}

/** Replaces what the schedule sets itself; agent defaults apply on top. */
export async function updateScheduleDependencies(
  scheduleId: number,
  dependencies: ScheduleDependencyInput[],
): Promise<ScheduleDependenciesResponse> {
  const response = await apiClient.put<ScheduleDependenciesResponse>(
    `/schedules/${scheduleId}/dependencies`,
    { dependencies },
  )
  return response.data
}

export async function getAgentDependencies(
  hostname: string,
  domain?: string | null,
): Promise<number[]> {
  const response = await apiClient.get<AgentDependenciesBody>(
    `/agents/${encodeURIComponent(hostname)}/dependencies`,
    { params: domainParams(domain) },
  )
  return response.data.dependency_host_ids
}

export async function updateAgentDependencies(
  hostname: string,
  dependencyHostIds: number[],
  domain?: string | null,
): Promise<number[]> {
  const response = await apiClient.put<AgentDependenciesBody>(
    `/agents/${encodeURIComponent(hostname)}/dependencies`,
    { dependency_host_ids: dependencyHostIds },
    { params: domainParams(domain) },
  )
  return response.data.dependency_host_ids
}
