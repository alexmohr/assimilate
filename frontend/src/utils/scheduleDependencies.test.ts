// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  dependencyCheckOutcomeText,
  dependencyWaitBadge,
  dependencyWaitNote,
  normalizeScheduleDependencies,
  summarizeScheduleDependencies,
} from './scheduleDependencies'
import type { DependencyWaitResponse } from '../types/generated'

function wait(over: Partial<DependencyWaitResponse> = {}): DependencyWaitResponse {
  return {
    schedule_id: 1,
    schedule_name: 'Nightly',
    agent_id: 10,
    hostname: 'web-01',
    dependency_host_id: 5,
    dependency_name: 'nas-media',
    pending_for: '2026-08-18T02:00:00Z',
    last_probe_at: null,
    next_probe_at: null,
    give_up_at: null,
    catching_up: false,
    ...over,
  }
}

const OUTCOME = { probed: 1, reachable: 0, started: 0, abandoned: 0, dropped: 0 }

describe('scheduleDependencies', () => {
  it('names one waiting dependency, and counts several', () => {
    expect(dependencyWaitBadge([])).toBeNull()
    expect(dependencyWaitBadge([wait(), wait({ agent_id: 11 })])).toBe('Waiting for nas-media')
    expect(dependencyWaitBadge([wait(), wait({ dependency_name: 'files-01' })])).toBe(
      'Waiting for 2 dependencies',
    )
  })

  it('says when a wait is next checked and given up on', () => {
    const soon = new Date(Date.now() + 5 * 60_000 + 30_000).toISOString()
    expect(dependencyWaitNote(wait({ next_probe_at: soon }))).toBe('Next check in 5m')
    expect(dependencyWaitNote(wait({ give_up_at: soon }))).toBe('Gives up in 5m')
    expect(dependencyWaitNote(wait({ catching_up: true, next_probe_at: soon }))).toBe(
      'Catching up now',
    )
  })

  it('reports what Check now found', () => {
    expect(dependencyCheckOutcomeText('nas', OUTCOME)).toBe('nas is still not answering')
    expect(dependencyCheckOutcomeText('nas', { ...OUTCOME, reachable: 1, started: 2 })).toBe(
      'nas is back - catching up 2 runs',
    )
    expect(dependencyCheckOutcomeText('nas', { ...OUTCOME, reachable: 1, started: 1 })).toBe(
      'nas is back - catching up 1 run',
    )
    expect(dependencyCheckOutcomeText('nas', { ...OUTCOME, reachable: 1 })).toBe(
      'nas is back, but each schedule runs again soon enough on its own',
    )
    expect(dependencyCheckOutcomeText('nas', { ...OUTCOME, probed: 0 })).toBe(
      'Nothing is waiting on nas',
    )
    expect(dependencyCheckOutcomeText('nas', { ...OUTCOME, probed: 0, abandoned: 1 })).toContain(
      '1 run was past its window',
    )
    expect(dependencyCheckOutcomeText('nas', { ...OUTCOME, probed: 0, abandoned: 3 })).toContain(
      '3 runs were past their window',
    )
  })

  it('lists a dependency once however many agents need it', () => {
    const summary = summarizeScheduleDependencies(
      [
        {
          agent_id: 1,
          dependency_host_id: 5,
          dependency_name: 'nas',
          source: 'agent_default',
          last_check_reachable: true,
        },
        {
          agent_id: 2,
          dependency_host_id: 5,
          dependency_name: 'nas',
          source: 'schedule',
          last_check_reachable: true,
        },
      ],
      (id) => `agent-${id}`,
    )
    expect(summary).toEqual([
      {
        id: 5,
        name: 'nas',
        lastCheckReachable: true,
        appliesTo: 'agent-1 (agent defaults), agent-2',
      },
    ])
  })

  it('reads an answer without lists as one with none', () => {
    expect(normalizeScheduleDependencies(null)).toEqual({ dependencies: [], waiting: [] })
    expect(normalizeScheduleDependencies({})).toEqual({ dependencies: [], waiting: [] })
  })
})
