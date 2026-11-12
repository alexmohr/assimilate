// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import DatabaseDumpMismatch from './DatabaseDumpMismatch.vue'
import type { AgentRow } from '../types/agent'

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
    database_backup_enabled: false,
    ...extra,
  } as unknown as AgentRow
}

function render(hosts: AgentRow[]): ReturnType<typeof renderWithPlugins> {
  return renderWithPlugins(DatabaseDumpMismatch, { props: { hosts } })
}

function text(wrapper: ReturnType<typeof renderWithPlugins>): string {
  return wrapper.text().replace(/\s+/g, ' ')
}

describe('DatabaseDumpMismatch', () => {
  it('announces itself as a status, naming the one host in the singular', () => {
    const wrapper = render([host(1, 'db-01')])
    const bar = wrapper.get('[data-testid="database-dump-mismatch"]')
    expect(bar.attributes('role')).toBe('status')
    expect(wrapper.get('.notice-bar-title').text()).toBe('No databases will be dumped on 1 host')
    expect(text(wrapper)).toContain(
      'This schedule asks for database dumps, but database dumps are turned off on db-01.',
    )
    expect(text(wrapper)).toContain(
      "Their backups run without them until dumps are allowed in each host's database settings.",
    )
  })

  it('counts several hosts in the plural', () => {
    const wrapper = render([host(1, 'db-01'), host(2, 'db-02'), host(3, 'db-03')])
    expect(wrapper.get('.notice-bar-title').text()).toBe('No databases will be dumped on 3 hosts')
  })

  it.each([
    [['db-01', 'db-02'], 'turned off on db-01 and db-02.'],
    [['db-01', 'db-02', 'db-03'], 'turned off on db-01, db-02 and db-03.'],
    [['a', 'b', 'c', 'd'], 'turned off on a, b, c and d.'],
  ])('joins %j as a sentence', (names, sentence) => {
    const wrapper = render(names.map((n, i) => host(i + 1, n)))
    expect(text(wrapper)).toContain(sentence)
  })

  it("links each host to its database settings, keeping a host's domain", () => {
    const wrapper = render([host(1, 'db-01'), host(2, 'web 02', { domain: 'corp.example' })])
    const links = wrapper.findAll('a.dump-mismatch-host')
    expect(links.map((l) => l.attributes('href'))).toEqual([
      '/agents/db-01?tab=sources&section=databases',
      '/agents/web%2002?tab=sources&section=databases&domain=corp.example',
    ])
  })

  it('names a host by its display name when it has one', () => {
    const wrapper = render([host(1, 'db-01', { display_name: 'Orders database' })])
    expect(wrapper.get('a.dump-mismatch-host').text()).toBe('Orders database')
    expect(wrapper.get('a.dump-mismatch-host').attributes('href')).toBe(
      '/agents/db-01?tab=sources&section=databases',
    )
  })
})
