// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { VueWrapper } from '@vue/test-utils'
import type {
  DependencyAvailabilityResponse,
  DependencyHostResponse,
  DependencyPowerResponse,
  DependencyWaitResponse,
} from '../types/generated'

/** Wake settings of a dependency with its own, switched off. */
export function dependencyPower(
  overrides: Partial<DependencyPowerResponse> = {},
): DependencyPowerResponse {
  return {
    repo_host: null,
    wake_enabled: false,
    wake_mac_address: null,
    wake_broadcast_address: null,
    wake_timeout_seconds: 180,
    effective_wake_enabled: false,
    effective_wake_mac_address: null,
    effective_wake_broadcast_address: null,
    effective_wake_timeout_seconds: 180,
    ...overrides,
  }
}

/** An SMB dependency that answered its last check. */
export function dependencyHost(
  overrides: Partial<DependencyHostResponse> = {},
): DependencyHostResponse {
  return {
    id: 3,
    name: 'nas-media',
    address: 'nas-media.lan',
    port: 445,
    description: 'Media share mounted on media-store-01',
    power: dependencyPower(),
    intermittent: false,
    catch_up_recheck_minutes: 15,
    catch_up_give_up_minutes: 0,
    last_checked_at: null,
    last_check_reachable: true,
    schedule_count: 2,
    agent_default_count: 1,
    waiting_count: 0,
    ...overrides,
  }
}

export function dependencyWait(
  overrides: Partial<DependencyWaitResponse> = {},
): DependencyWaitResponse {
  return {
    schedule_id: 4,
    schedule_name: 'Media share nightly',
    agent_id: 7,
    hostname: 'media-store-01',
    dependency_host_id: 3,
    dependency_name: 'nas-media',
    pending_for: '2026-09-22T02:00:00Z',
    last_probe_at: '2026-09-22T03:06:00Z',
    next_probe_at: '2026-09-22T03:21:00Z',
    give_up_at: '2026-09-25T02:00:00Z',
    catching_up: false,
    ...overrides,
  }
}

export function dependencyAvailability(
  overrides: Partial<DependencyAvailabilityResponse> = {},
): DependencyAvailabilityResponse {
  return {
    intermittent: true,
    catch_up_recheck_minutes: 15,
    catch_up_give_up_minutes: 24 * 60,
    waiting: [dependencyWait()],
    ...overrides,
  }
}

/** A card's `.info-grid`, term to value, for asserting what its view shows. */
export function infoGridRows(wrapper: Pick<VueWrapper, 'findAll'>): Record<string, string> {
  const values = wrapper.findAll('dd').map((d) => d.text())
  return Object.fromEntries(wrapper.findAll('dt').map((d, i) => [d.text(), values[i] ?? '']))
}
