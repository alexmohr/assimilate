// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import SchedulePowerTab from './SchedulePowerTab.vue'
import type { ScheduleWakeOverride } from '../types/generated'
import type { AgentRow } from '../types/agent'
import type { Repo } from '../types/repo'

function agent(id: number, hostname: string, wake: Partial<Record<string, unknown>>): AgentRow {
  return {
    id,
    hostname,
    power: {
      wake: {
        wake_enabled: false,
        wake_mac_address: null,
        wake_broadcast_address: null,
        wake_timeout_seconds: 180,
        shutdown_after_backup: false,
        ...wake,
      },
    },
  } as unknown as AgentRow
}

const REPO = {
  id: 20,
  ssh_host: 'backup-nas.lan',
  power: {
    wake_enabled: false,
    wake_mac_address: '9C:B6:D0:1A:44:7F',
    wake_broadcast_address: null,
    wake_timeout_seconds: 180,
    shutdown_after_backup: false,
  },
} as unknown as Repo

/** A second target, on a host of its own. */
const OFFSITE_REPO = {
  id: 21,
  ssh_host: 'offsite-nas.lan',
  power: {
    wake_enabled: false,
    wake_mac_address: 'AA:BB:CC:DD:EE:01',
    wake_broadcast_address: null,
    wake_timeout_seconds: 180,
    shutdown_after_backup: false,
  },
} as unknown as Repo

/** A third target that shares `REPO`'s machine. */
const SAME_HOST_REPO = {
  id: 22,
  ssh_host: 'backup-nas.lan',
  power: {
    wake_enabled: false,
    wake_mac_address: '9C:B6:D0:1A:44:7F',
    wake_broadcast_address: null,
    wake_timeout_seconds: 180,
    shutdown_after_backup: false,
  },
} as unknown as Repo

/** Wakes by default, with a MAC address and a shutdown configured. */
const WAKING = agent(10, 'web-server-01', {
  wake_enabled: true,
  wake_mac_address: '3C:97:0E:2B:9A:44',
  shutdown_after_backup: true,
})
/** Never wakes, and has nothing to wake it with. */
const NO_MAC = agent(11, 'media-store-01', {})

function mount(wakeOverride: ScheduleWakeOverride, props: Record<string, unknown> = {}) {
  return renderWithPlugins(SchedulePowerTab, {
    props: {
      wakeOverride,
      agents: [WAKING, NO_MAC],
      repos: [REPO],
      selectedAgentIds: [10, 11],
      selectedRepoIds: [20],
      canSeeWakeDetails: true,
      ...props,
    },
  })
}

/** The badge label rendered for each host, in the order the rows appear. */
function labels(wrapper: ReturnType<typeof mount>): string[] {
  return wrapper.findAll('.badge').map((b) => b.text())
}

describe('SchedulePowerTab', () => {
  it('offers the three states, with the host default selected by default', () => {
    const wrapper = mount('host_default')
    const options = wrapper.findAll('.segmented-option')

    expect(options.map((o) => o.text())).toEqual(['Host default', 'Enabled', 'Disabled'])
    expect(options[0]!.attributes('aria-checked')).toBe('true')
  })

  it('emits the chosen state back to its model', async () => {
    const wrapper = mount('host_default')
    await wrapper.findAll('.segmented-option')[2]!.trigger('click')

    expect(wrapper.emitted('update:wakeOverride')).toEqual([['disabled']])
  })

  // Each row of the host-setting x job-setting matrix, over one host that
  // wakes by default, one with no MAC address, and the repository host.
  it('echoes each host under the host default', () => {
    expect(labels(mount('host_default'))).toEqual(['Wakes', 'Not woken', 'Not woken'])
  })

  it('wakes every host it can under enabled, and flags the one it cannot', () => {
    expect(labels(mount('enabled'))).toEqual(['Wakes', 'Cannot wake', 'Wakes'])
  })

  it('wakes nothing under disabled, including a host that wakes by default', () => {
    expect(labels(mount('disabled'))).toEqual(['Not woken', 'Not woken', 'Not woken'])
  })

  it('says a host is woken for this job only where the host itself would not', () => {
    expect(mount('enabled').text()).toContain('Woken for this job only')
  })

  it('says a shutdown follows a wake this run is responsible for', () => {
    expect(mount('host_default').text()).toContain('Shut down afterwards')
  })

  // The API redacts a host's MAC address below operator, so a viewer who
  // cannot see one must not be told the host has none.
  it('does not claim a host cannot be woken when the viewer cannot see MAC addresses', () => {
    const wrapper = mount('enabled', { canSeeWakeDetails: false })

    expect(labels(wrapper)).toEqual(['Wakes', 'Wakes', 'Wakes'])
  })

  it('asks for targets first when none are selected', () => {
    const wrapper = mount('host_default', { selectedAgentIds: [], selectedRepoIds: [] })

    expect(wrapper.findAll('.badge')).toHaveLength(0)
    expect(wrapper.text()).toContain("Pick this job's agents and repository first")
  })

  /** Every target's host is woken in turn, so reporting only the primary
      would under-report what the run actually powers on. */
  it('reports the host of every target repository', () => {
    const wrapper = mount('host_default', {
      repos: [REPO, OFFSITE_REPO],
      selectedRepoIds: [20, 21],
    })

    expect(wrapper.text()).toContain('backup-nas.lan')
    expect(wrapper.text()).toContain('offsite-nas.lan')
  })

  /** Waking is per machine, so two targets on one host are one host. */
  it('lists a host shared by two targets once', () => {
    const wrapper = mount('host_default', {
      repos: [REPO, SAME_HOST_REPO],
      selectedRepoIds: [20, 22],
    })

    const hosts = wrapper.findAll('.badge').length
    expect(hosts).toBe(3)
    expect(wrapper.text().match(/backup-nas\.lan/g)).toHaveLength(1)
  })

  it('names every selected host and its role', () => {
    const text = mount('host_default').text()

    expect(text).toContain('web-server-01')
    expect(text).toContain('media-store-01')
    expect(text).toContain('backup-nas.lan')
    expect(text).toContain('repository host')
  })
})
