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
  const button = wrapper.findAll('button').find((b) => b.text().trim() === 'Run now')
  if (!button) throw new Error('no Run now button')
  await button.trigger('click')
  await flushPromises()
}

describe('AgentSchedulesTab', () => {
  beforeEach(() => {
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.post).mockResolvedValue({ data: {} } as never)
  })

  it('renders one row per schedule', () => {
    expect(mount().findAll('.rows .agent-row')).toHaveLength(1)
    expect(mount().text()).toContain('Nightly')
  })

  // A schedule can target several hosts. "Run now" pressed on one host's page
  // must not kick off a backup on the others.
  it('restricts Run now to this agent', async () => {
    const wrapper = mount()
    await clickRunNow(wrapper)

    expect(apiClient.post).toHaveBeenCalledWith('/schedules/100/run', { agent_ids: [7] })
  })

  it.each([['Open'], ['Nightly']])('opens the schedule from %s', async (label) => {
    const wrapper = mount()
    const target =
      label === 'Open'
        ? wrapper.findAll('button').find((b) => b.text().trim() === 'Open')
        : wrapper.find('.agent-row-name')
    await target!.trigger('click')

    expect(wrapper.emitted('open')).toHaveLength(1)
  })

  // An unnamed schedule is identified by the repository it writes to.
  it('falls back to the repository name for an unnamed schedule', () => {
    const wrapper = mount({ schedules: [schedule({ name: '' })] })
    expect(wrapper.find('.agent-row-name').text()).toBe('server-daily')
  })

  it('surfaces a failed run request', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('repo locked'))
    const wrapper = mount()
    await clickRunNow(wrapper)

    expect(wrapper.find('.form-error').exists()).toBe(true)
  })

  it('cannot run a disabled schedule', () => {
    const wrapper = mount({ schedules: [schedule({ enabled: false })] })
    const button = wrapper.findAll('button').find((b) => b.text().trim() === 'Run now')
    expect(button!.attributes('disabled')).toBeDefined()
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
