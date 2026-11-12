// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'
import { flushPromises } from '@vue/test-utils'
import { pushWs, resetWsMock } from '../test-utils/wsMock'
import type { ReportRow } from '../types/report'
import type { RunEventResponse } from '../types/generated'

vi.mock('./useWebSocket', () => import('../test-utils/wsMock'))
vi.mock('../api/runs', () => ({ getRunEvents: vi.fn() }))
vi.mock('../utils/logger', () => ({ logger: { error: vi.fn() } }))

import { getRunEvents } from '../api/runs'
import { useRunEvents } from './useRunEvents'

const REPORT = { id: 1, run_id: 'run-1', agent_id: 7, repo_id: 3 } as ReportRow

function stored(id: number, message: string): RunEventResponse {
  return {
    id,
    run_id: 'run-1',
    target: 'web-01',
    event_type: 'wake_sent',
    message,
    occurred_at: '2026-01-01T10:00:00Z',
  }
}

/** A RunEvent push for `report`'s copy unless told otherwise. */
function live(message: string, overrides: Record<string, unknown> = {}): void {
  pushWs('RunEvent', {
    run_id: 'run-1',
    agent_id: 7,
    repo_id: 3,
    target: 'web-01',
    event_type: 'host_reachable',
    message,
    occurred_at: '2026-01-01T10:01:00Z',
    ...overrides,
  })
}

describe('useRunEvents', () => {
  beforeEach(() => {
    vi.resetAllMocks()
    resetWsMock()
  })

  it('fetches the run timeline for this copy only once it is expanded', async () => {
    vi.mocked(getRunEvents).mockResolvedValue([stored(1, 'Sent Wake-on-LAN')])
    const expanded = ref(false)
    const state = useRunEvents(
      () => REPORT,
      () => expanded.value,
    )
    await flushPromises()
    expect(getRunEvents).not.toHaveBeenCalled()

    expanded.value = true
    await flushPromises()

    expect(getRunEvents).toHaveBeenCalledWith('run-1', 7, 3)
    expect(state.eventsFetched.value).toBe(true)
    expect(state.loadingEvents.value).toBe(false)
    expect(state.runEvents.value.map((e) => e.message)).toEqual(['Sent Wake-on-LAN'])
  })

  it('fetches straight away for a run that arrives already expanded', async () => {
    vi.mocked(getRunEvents).mockResolvedValue([])
    const state = useRunEvents(
      () => REPORT,
      () => true,
    )
    await flushPromises()

    expect(getRunEvents).toHaveBeenCalledTimes(1)
    expect(state.eventsFetched.value).toBe(true)
    expect(state.runEvents.value).toEqual([])
  })

  it('has nothing to fetch for a report without a run id', async () => {
    useRunEvents(
      () => ({ ...REPORT, run_id: null }) as ReportRow,
      () => true,
    )
    await flushPromises()

    expect(getRunEvents).not.toHaveBeenCalled()
  })

  it('appends live events of this copy once fetched, ignoring a sibling target', async () => {
    vi.mocked(getRunEvents).mockResolvedValue([stored(1, 'Sent Wake-on-LAN')])
    const state = useRunEvents(
      () => REPORT,
      () => true,
    )
    await flushPromises()

    live('Host answered')
    live('Other host answered', { agent_id: 8 })
    live('Other repo', { repo_id: 4 })
    live('Other run', { run_id: 'run-2' })

    expect(state.runEvents.value.map((e) => e.message)).toEqual([
      'Sent Wake-on-LAN',
      'Host answered',
    ])
    // A live event has no row id; it gets a negative key of its own.
    expect(state.runEvents.value[1]!.id).toBeLessThan(0)
  })

  it('keeps a live event that lands while the fetch is in flight', async () => {
    let resolve: (events: RunEventResponse[]) => void = () => undefined
    vi.mocked(getRunEvents).mockReturnValue(new Promise((r) => (resolve = r)))
    const state = useRunEvents(
      () => REPORT,
      () => true,
    )
    expect(state.loadingEvents.value).toBe(true)

    live('Host answered')
    resolve([stored(1, 'Sent Wake-on-LAN')])
    await flushPromises()

    expect(state.runEvents.value.map((e) => e.message)).toEqual([
      'Sent Wake-on-LAN',
      'Host answered',
    ])
  })

  it('drops a live event for a row that was never opened', async () => {
    const state = useRunEvents(
      () => REPORT,
      () => false,
    )
    live('Host answered')
    await flushPromises()

    expect(state.runEvents.value).toEqual([])
  })

  it('says the timeline could not be loaded when the fetch fails', async () => {
    vi.mocked(getRunEvents).mockRejectedValue(new Error('boom'))
    const state = useRunEvents(
      () => REPORT,
      () => true,
    )
    await flushPromises()

    expect(state.eventsError.value).toBe(true)
    expect(state.eventsFetched.value).toBe(false)
    expect(state.loadingEvents.value).toBe(false)
  })
})
