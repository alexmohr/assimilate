// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { VueWrapper } from '@vue/test-utils'
import type { DependencyHostResponse } from '../types/generated'

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
    machine_id: 30,
    repo_host: null,
    intermittent: false,
    wake_enabled: false,
    last_checked_at: null,
    last_check_reachable: true,
    schedule_count: 2,
    agent_default_count: 1,
    waiting_count: 0,
    ...overrides,
  }
}

/** A card's `.info-grid`, term to value, for asserting what its view shows. */
export function infoGridRows(wrapper: Pick<VueWrapper, 'findAll'>): Record<string, string> {
  const values = wrapper.findAll('dd').map((d) => d.text())
  return Object.fromEntries(wrapper.findAll('dt').map((d, i) => [d.text(), values[i] ?? '']))
}
