// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import AgentSchedulesTab from './AgentSchedulesTab.vue'
import type { AgentRow } from '../types/agent'
import type { ScheduleRow } from '../types/schedule'
import type { ScheduleHealthEntry } from '../utils/scheduleHealth'

vi.mock('../api/client', () => ({
  apiClient: { post: vi.fn() },
}))

vi.mock('../utils/error', () => ({
  extractError: (_e: unknown, fallback?: string) => fallback ?? 'Unknown error',
}))

const AGENT = { id: 7, hostname: 'web-01', is_imported: false } as unknown as AgentRow

function schedule(over: Record<string, unknown> = {}): ScheduleRow {
  return {
    id: 100,
    repo_id: 10,
    name: 'Nightly',
    target_hostnames: ['web-01'],
    cron_expression: '0 2 * * *',
    enabled: true,
    next_run_at: '2026-06-02T02:00:00Z',
    last_run_at: '2026-06-01T02:00:00Z',
    ...over,
  } as unknown as ScheduleRow
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentSchedulesTab, {
    props: {
      agent: AGENT,
      schedules: [schedule()],
      health: [] as ScheduleHealthEntry[],
      highlightOverdue: false,
      repoNameFor: () => 'server-daily',
      ...props,
    },
  })
}

async function clickRunNow(wrapper: ReturnType<typeof mount>) {
  await wrapper.get('button[title="Run now"]').trigger('click')
  await flushPromises()
}

describe('AgentSchedulesTab', () => {
  beforeEach(() => {
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.post).mockResolvedValue({ data: {} } as never)
  })

  it('renders one row per schedule', () => {
    const rows = mount().findAll('.sched-row[data-schedule-id]')
    expect(rows).toHaveLength(1)
    expect(rows[0].get('.sched-title').text()).toBe('Nightly')
  })

  // A schedule can target several hosts. "Run now" pressed on one host's page
  // must not kick off a backup on the others.
  it('restricts Run now to this agent', async () => {
    const wrapper = mount()
    await clickRunNow(wrapper)

    expect(apiClient.post).toHaveBeenCalledWith('/schedules/100/run', { agent_ids: [7] })
  })

  // The row itself is the link to the schedule; there is no separate Open
  // button any more.
  it.each([
    ['a click', 'click'],
    ['Enter', 'keydown.enter'],
  ])('opens the schedule from the row on %s', async (_label, event) => {
    const wrapper = mount()
    await wrapper.get('.sched-row[data-schedule-id="100"]').trigger(event)

    expect(wrapper.emitted('open')).toEqual([[expect.objectContaining({ id: 100 })]])
  })

  it('does not open the schedule when Run now is pressed', async () => {
    const wrapper = mount()
    await clickRunNow(wrapper)
    expect(wrapper.emitted('open')).toBeUndefined()
  })

  // An unnamed schedule is identified by the repository it writes to.
  it('falls back to the repository name for an unnamed schedule', () => {
    const wrapper = mount({ schedules: [schedule({ name: '' })] })
    expect(wrapper.get('.sched-title').text()).toBe('server-daily')
    expect(wrapper.get('button[title="Run now"]').attributes('aria-label')).toBe(
      'Run server-daily now',
    )
  })

  it('surfaces a failed run request', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('repo locked'))
    const wrapper = mount()
    await clickRunNow(wrapper)

    expect(wrapper.find('.form-error').exists()).toBe(true)
  })

  it('cannot run a disabled schedule', () => {
    const wrapper = mount({ schedules: [schedule({ enabled: false })] })
    expect(wrapper.get('button[title="Run now"]').attributes('disabled')).toBeDefined()
  })

  it('offers a way to add a schedule when there are none', () => {
    const wrapper = mount({ schedules: [] })
    expect(wrapper.find('.empty-title').text()).toBe('No schedules yet')
  })

  describe('imported hosts', () => {
    const IMPORTED = { ...AGENT, is_imported: true }

    // The tab is kept rather than hidden so the tab bar is the same shape on
    // every host - and so this is where the emptiness ends once adopted.
    it('explains why an imported host has no schedules', () => {
      const wrapper = mount({ agent: IMPORTED, schedules: [] })
      const description = wrapper.find('.empty-description').text()
      expect(description).toContain('reconstructed from archives')
      expect(description).toContain('Adopt')
      expect(description).toContain('merge it')
    })

    it('does not offer to add a schedule to a host with no agent', () => {
      const wrapper = mount({ agent: IMPORTED, schedules: [] })
      expect(wrapper.text()).not.toContain('Add schedule')
    })
  })
})
