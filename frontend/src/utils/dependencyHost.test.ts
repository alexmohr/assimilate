// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { dependencyHost } from '../test-utils/dependencyFixtures'
import type { MachineWaitResponse } from '../types/generated'
import {
  checkDescription,
  countLabel,
  dependencyWaits,
  matchesDependencyFilter,
  matchesDependencySearch,
  presetForPort,
  presetPort,
  protocolLabel,
  reachabilityBadge,
  sortDependencies,
  testResultText,
  waitingRunsText,
} from './dependencyHost'
import { isDependencyHostSection } from './dependencyHostSettings'

describe('dependency host helpers', () => {
  it('names the well-known ports and calls the rest TCP', () => {
    expect([445, 2049, 22, 8443].map(protocolLabel)).toEqual(['SMB', 'NFS', 'SSH', 'TCP'])
    expect(checkDescription(445)).toBe('SMB, TCP port 445')
    expect(checkDescription(8443)).toBe('TCP port 8443')
  })

  it('maps presets to ports and back', () => {
    expect(presetPort('nfs')).toBe(2049)
    expect(presetPort('other')).toBeNull()
    expect(presetForPort(22)).toBe('ssh')
    expect(presetForPort(8443)).toBe('other')
  })

  it('tells a dependency never checked apart from one that did not answer', () => {
    expect(reachabilityBadge(true)).toEqual({ tone: 'success', label: 'Answering' })
    expect(reachabilityBadge(false)).toEqual({ tone: 'warning', label: 'Not answering' })
    expect(reachabilityBadge(null)).toEqual({ tone: 'neutral', label: 'Not checked yet' })
  })

  it('reports a connection test', () => {
    expect(
      testResultText({ reachable: true, address: 'nas.lan', port: 445, timeout_seconds: 5 }),
    ).toBe('nas.lan answered on port 445')
    expect(
      testResultText({ reachable: false, address: 'nas.lan', port: 445, timeout_seconds: 5 }),
    ).toBe('nas.lan did not answer on port 445 within 5 seconds')
  })

  // The server decides how long a probe waits; the sentence must not repeat
  // a number of its own that could drift from it.
  it("states the server's probe timeout rather than a fixed one", () => {
    expect(
      testResultText({ reachable: false, address: 'nas.lan', port: 445, timeout_seconds: 12 }),
    ).toBe('nas.lan did not answer on port 445 within 12 seconds')
  })

  it('parses only the sections the page has', () => {
    expect(isDependencyHostSection('used-by')).toBe(true)
    expect(isDependencyHostSection('repositories')).toBe(false)
    expect(isDependencyHostSection(undefined)).toBe(false)
  })

  it('filters by what the last check found and by waiting runs', () => {
    const ok = dependencyHost()
    const silent = dependencyHost({ last_check_reachable: false })
    const unchecked = dependencyHost({ last_check_reachable: null })
    const waited = dependencyHost({ waiting_count: 2 })
    expect([ok, silent, unchecked, waited].map((h) => matchesDependencyFilter(h, 'all'))).toEqual([
      true,
      true,
      true,
      true,
    ])
    expect(matchesDependencyFilter(silent, 'not_answering')).toBe(true)
    expect(matchesDependencyFilter(unchecked, 'not_answering')).toBe(false)
    expect(matchesDependencyFilter(unchecked, 'not_checked')).toBe(true)
    expect(matchesDependencyFilter(ok, 'not_checked')).toBe(false)
    expect(matchesDependencyFilter(waited, 'waiting')).toBe(true)
    expect(matchesDependencyFilter(ok, 'waiting')).toBe(false)
  })

  it('sorts problems first, then by name, without touching its input', () => {
    const input = [
      dependencyHost({ name: 'b-ok' }),
      dependencyHost({ name: 'a-ok' }),
      dependencyHost({ name: 'unchecked', last_check_reachable: null }),
      dependencyHost({ name: 'waited', waiting_count: 1 }),
      dependencyHost({ name: 'silent', last_check_reachable: false, waiting_count: 1 }),
    ]
    expect(sortDependencies(input).map((h) => h.name)).toEqual([
      'silent',
      'waited',
      'unchecked',
      'a-ok',
      'b-ok',
    ])
    expect(input[0].name).toBe('b-ok')
  })

  it('searches the name and the address, ignoring case and blank queries', () => {
    const host = dependencyHost({ name: 'nas-media', address: '10.0.0.5' })
    expect(matchesDependencySearch(host, '  ')).toBe(true)
    expect(matchesDependencySearch(host, 'NAS')).toBe(true)
    expect(matchesDependencySearch(host, '10.0')).toBe(true)
    expect(matchesDependencySearch(host, 'files')).toBe(false)
  })

  it("keeps only a machine's waits on the dependency itself", () => {
    const base: MachineWaitResponse = {
      role: 'dependency',
      schedule_id: 1,
      schedule_name: 's',
      hostname: 'h',
      repo_id: null,
      repo_name: null,
      dependency_name: 'nas-media',
      pending_for: '2026-11-04T03:00:00Z',
      last_probe_at: null,
      next_probe_at: null,
      give_up_at: null,
    }
    const waits: MachineWaitResponse[] = [
      base,
      { ...base, dependency_name: 'files-01' },
      { ...base, role: 'repo_host', dependency_name: null },
      { ...base, role: 'agent', dependency_name: null },
    ]
    expect(dependencyWaits(waits, 'nas-media')).toEqual([base])
  })

  it('counts in words', () => {
    expect(waitingRunsText(1)).toBe('1 run waiting')
    expect(waitingRunsText(3)).toBe('3 runs waiting')
    expect(countLabel(1, 'schedule', 'schedules')).toBe('1 schedule')
    expect(countLabel(0, 'schedule', 'schedules')).toBe('0 schedules')
  })
})
