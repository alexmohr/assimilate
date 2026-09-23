// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import { domainParams } from '../utils/agent'
import type { HostAvailabilityResponse, RepoCatchUpCheckResponse } from '../types/generated'

/**
 * A host's "When the host is offline" settings, as saved.
 *
 * `catch_up_recheck_minutes` only exists for a repository: an agent announces
 * its own return by reconnecting, so there is nothing to ask it on an interval.
 */
export interface HostAvailabilityUpdate {
  intermittent: boolean
  catch_up_recheck_minutes?: number
  catch_up_give_up_minutes: number
}

/**
 * The three calls an availability section makes, bound to one agent or one
 * repository - so the section itself does not need to know which it is.
 * `check` is absent for an agent: it cannot be asked, only waited for.
 */
export interface HostAvailabilityApi {
  load: () => Promise<HostAvailabilityResponse>
  save: (data: HostAvailabilityUpdate) => Promise<HostAvailabilityResponse>
  check?: () => Promise<RepoCatchUpCheckResponse>
}

export function repoAvailabilityApi(repoId: number): HostAvailabilityApi {
  return {
    load: async () =>
      (await apiClient.get<HostAvailabilityResponse>(`/repos/${repoId}/availability`)).data,
    save: async (data) =>
      (await apiClient.put<HostAvailabilityResponse>(`/repos/${repoId}/availability`, data)).data,
    check: async () =>
      (await apiClient.post<RepoCatchUpCheckResponse>(`/repos/${repoId}/availability/check`)).data,
  }
}

export function agentAvailabilityApi(
  hostname: string,
  domain?: string | null,
): HostAvailabilityApi {
  const config = { params: domainParams(domain) }
  return {
    load: async () =>
      (await apiClient.get<HostAvailabilityResponse>(`/agents/${hostname}/availability`, config))
        .data,
    save: async (data) =>
      (
        await apiClient.put<HostAvailabilityResponse>(
          `/agents/${hostname}/availability`,
          {
            intermittent: data.intermittent,
            catch_up_give_up_minutes: data.catch_up_give_up_minutes,
          },
          config,
        )
      ).data,
  }
}
