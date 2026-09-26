// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { apiClient } from './client'
import { domainParams } from '../utils/agent'
import type { HostAvailabilityResponse, RepoCatchUpCheckResponse } from '../types/generated'

/**
 * A host's "When the host is offline" settings, as saved.
 *
 * `catch_up_recheck_minutes` only exists for a repository host: an agent announces
 * its own return by reconnecting, so there is nothing to ask it on an interval.
 */
export interface HostAvailabilityUpdate {
  intermittent: boolean
  catch_up_recheck_minutes?: number
  catch_up_give_up_minutes: number
}

/**
 * The three calls an availability section makes, bound to one agent or one
 * repository host - so the section itself does not need to know which it is.
 * `check` is absent for an agent: it cannot be asked, only waited for.
 */
export interface HostAvailabilityApi {
  /**
   * Which host these calls are bound to. The object itself is rebuilt on every
   * render of the page that makes it, so this - not its identity - is what
   * tells the section it is now showing a different host.
   */
  host: string
  load: () => Promise<HostAvailabilityResponse>
  save: (data: HostAvailabilityUpdate) => Promise<HostAvailabilityResponse>
  check?: () => Promise<RepoCatchUpCheckResponse>
}

export function repoHostAvailabilityApi(repoHostId: number): HostAvailabilityApi {
  return {
    host: `repo-host:${repoHostId}`,
    load: async () =>
      (await apiClient.get<HostAvailabilityResponse>(`/repo-hosts/${repoHostId}/availability`))
        .data,
    save: async (data) =>
      (
        await apiClient.put<HostAvailabilityResponse>(
          `/repo-hosts/${repoHostId}/availability`,
          data,
        )
      ).data,
    check: async () =>
      (
        await apiClient.post<RepoCatchUpCheckResponse>(
          `/repo-hosts/${repoHostId}/availability/check`,
        )
      ).data,
  }
}

/**
 * A repository's view of its host's "When the host is offline" settings, and
 * the schedules waiting on this repository. Read-only: the settings belong to
 * the repository host and are changed there.
 */
export async function getRepoAvailability(repoId: number): Promise<HostAvailabilityResponse> {
  return (await apiClient.get<HostAvailabilityResponse>(`/repos/${repoId}/availability`)).data
}

export function agentAvailabilityApi(
  hostname: string,
  domain?: string | null,
): HostAvailabilityApi {
  const config = { params: domainParams(domain) }
  return {
    host: `agent:${hostname}@${domain ?? ''}`,
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
