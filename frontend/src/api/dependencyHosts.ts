// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import { domainParams } from '../utils/agent'
import type {
  AgentDependenciesBody,
  DependencyAvailabilityResponse,
  DependencyHostResponse,
  DependencyTestResponse,
  DependencyUsageResponse,
  RepoCatchUpCheckResponse,
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

/**
 * How a dependency is woken. With `repo_host_id` set the repository host's wake
 * settings apply; the dependency's own are still saved, so switching back
 * restores them.
 */
export interface UpdateDependencyPowerRequest {
  repo_host_id: number | null
  wake_enabled: boolean
  wake_mac_address: string | null
  wake_broadcast_address: string | null
  wake_timeout_seconds: number
}

export interface UpdateDependencyAvailabilityRequest {
  intermittent: boolean
  catch_up_recheck_minutes: number
  catch_up_give_up_minutes: number
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

export async function updateDependencyHostPower(
  id: number,
  data: UpdateDependencyPowerRequest,
): Promise<DependencyHost> {
  const response = await apiClient.put<DependencyHost>(`/dependency-hosts/${id}/power`, data)
  return response.data
}

export async function getDependencyHostAvailability(
  id: number,
): Promise<DependencyAvailabilityResponse> {
  const response = await apiClient.get<DependencyAvailabilityResponse>(
    `/dependency-hosts/${id}/availability`,
  )
  return response.data
}

export async function updateDependencyHostAvailability(
  id: number,
  data: UpdateDependencyAvailabilityRequest,
): Promise<DependencyAvailabilityResponse> {
  const response = await apiClient.put<DependencyAvailabilityResponse>(
    `/dependency-hosts/${id}/availability`,
    data,
  )
  return response.data
}

/** Asks the dependency whether it is back and catches up every run waiting on it if so. */
export async function checkDependencyHostNow(id: number): Promise<RepoCatchUpCheckResponse> {
  const response = await apiClient.post<RepoCatchUpCheckResponse>(
    `/dependency-hosts/${id}/availability/check`,
  )
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
