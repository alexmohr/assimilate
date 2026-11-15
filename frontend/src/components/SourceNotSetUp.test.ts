// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import SourceNotSetUp from './SourceNotSetUp.vue'
import type { AgentRow } from '../types/agent'
import type { HostSourceKind } from '../composables/useHostSources'

function host(
  id: number,
  hostname: string,
  extra: { display_name?: string | null; domain?: string | null } = {},
): AgentRow {
  return {
    id,
    hostname,
    display_name: null,
    domain: null,
    ...extra,
  } as unknown as AgentRow
}

function render(
  hosts: AgentRow[],
  kind: HostSourceKind = 'databases',
): ReturnType<typeof renderWithPlugins> {
  return renderWithPlugins(SourceNotSetUp, { props: { hosts, kind } })
}

function text(wrapper: ReturnType<typeof renderWithPlugins>): string {
  return wrapper.text().replace(/\s+/g, ' ')
}

describe('SourceNotSetUp', () => {
  // Was DatabaseDumpMismatch, about hosts with dumps turned off. Hosts no
  // longer turn dumps off; a schedule gets none from a host that has no
  // database source set up, and the same holds for virtual machines.
  it('announces itself as a status, naming the one host in the singular', () => {
    const wrapper = render([host(1, 'db-01')])
    const bar = wrapper.get('[data-testid="source-not-set-up-databases"]')
    expect(bar.attributes('role')).toBe('status')
    expect(wrapper.get('.notice-bar-title').text()).toBe('No databases will be dumped on 1 host')
    expect(text(wrapper)).toContain('This schedule takes databases, but none are set up on db-01.')
    expect(text(wrapper)).toContain(
      "Scan or select them in each host's Sources tab, or turn databases off for this schedule.",
    )
  })

  it('says the same of virtual machines, under its own test id', () => {
    const wrapper = render([host(1, 'virt-01')], 'vms')
    expect(wrapper.find('[data-testid="source-not-set-up-vms"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="source-not-set-up-databases"]').exists()).toBe(false)
    expect(wrapper.get('.notice-bar-title').text()).toBe(
      'No virtual machines will be backed up on 1 host',
    )
    expect(text(wrapper)).toContain(
      'This schedule takes virtual machines, but none are set up on virt-01.',
    )
    expect(text(wrapper)).toContain('or turn virtual machines off for this schedule.')
  })

  it('counts several hosts in the plural', () => {
    const wrapper = render([host(1, 'db-01'), host(2, 'db-02'), host(3, 'db-03')])
    expect(wrapper.get('.notice-bar-title').text()).toBe('No databases will be dumped on 3 hosts')
  })

  it.each([
    [['db-01', 'db-02'], 'set up on db-01 and db-02.'],
    [['db-01', 'db-02', 'db-03'], 'set up on db-01, db-02 and db-03.'],
    [['a', 'b', 'c', 'd'], 'set up on a, b, c and d.'],
  ])('joins %j as a sentence', (names, sentence) => {
    const wrapper = render(names.map((n, i) => host(i + 1, n)))
    expect(text(wrapper)).toContain(sentence)
  })

  it.each<[HostSourceKind, string]>([
    ['databases', 'databases'],
    ['vms', 'vms'],
  ])(
    "links each host to its %s list on the Sources tab, keeping a host's domain",
    (kind, section) => {
      const wrapper = render(
        [host(1, 'db-01'), host(2, 'web 02', { domain: 'corp.example' })],
        kind,
      )
      const links = wrapper.findAll('a.dump-mismatch-host')
      expect(links.map((l) => l.attributes('href'))).toEqual([
        `/agents/db-01?tab=sources&section=${section}`,
        `/agents/web%2002?tab=sources&section=${section}&domain=corp.example`,
      ])
    },
  )

  it('names a host by its display name when it has one', () => {
    const wrapper = render([host(1, 'db-01', { display_name: 'Orders database' })])
    expect(wrapper.get('a.dump-mismatch-host').text()).toBe('Orders database')
    expect(wrapper.get('a.dump-mismatch-host').attributes('href')).toBe(
      '/agents/db-01?tab=sources&section=databases',
    )
  })
})
