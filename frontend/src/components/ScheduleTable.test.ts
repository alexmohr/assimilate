// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createMockRouter } from '../test-utils'
import ScheduleTable from './ScheduleTable.vue'
import type { ScheduleRow } from '../types/schedule'
import type { ScheduleHealthEntry } from '../utils/scheduleHealth'

function schedule(id: number, name: string): ScheduleRow {
  return {
    id,
    repo_id: 10,
    name,
    target_hostnames: ['web-01'],
    cron_expression: '0 2 * * *',
    enabled: true,
    next_run_at: '2026-06-02T02:00:00Z',
    last_run_at: '2026-06-01T02:00:00Z',
  } as unknown as ScheduleRow
}

function health(scheduleId: number, lastStatus: string): ScheduleHealthEntry {
  return {
    repo_id: 10,
    schedule_id: scheduleId,
    hostname: 'web-01',
    target_name: 'server-daily',
    last_status: lastStatus,
    last_backup_at: '2026-06-01T02:05:00Z',
    last_error_message: lastStatus === 'success' ? null : 'borg exited 2',
    is_overdue: false,
  } as unknown as ScheduleHealthEntry
}

function mountTable() {
  const router = createMockRouter()
  const push = vi.spyOn(router, 'push').mockResolvedValue(undefined)
  const wrapper = mount(ScheduleTable, {
    props: {
      schedules: [schedule(1, 'Nightly'), schedule(2, 'Hourly')],
      health: [health(1, 'failed'), health(2, 'success')],
      subtitleFor: () => 'server-daily',
      runningId: null,
    },
    global: { plugins: [router] },
  })
  return { wrapper, push }
}

function titles(wrapper: ReturnType<typeof mountTable>['wrapper']): string[] {
  return wrapper.findAll('[data-schedule-id] .sched-title').map((t) => t.text())
}

function pill(wrapper: ReturnType<typeof mountTable>['wrapper'], label: string) {
  const match = wrapper.findAll('.filter-pill').find((b) => b.text().startsWith(label))
  if (!match) throw new Error(`no filter pill "${label}"`)
  return match
}

describe('ScheduleTable', () => {
  it('narrows the rows to the picked filter', async () => {
    const { wrapper } = mountTable()
    expect(titles(wrapper)).toEqual(['Nightly', 'Hourly'])

    await pill(wrapper, 'Failing').trigger('click')
    expect(pill(wrapper, 'Failing').attributes('aria-pressed')).toBe('true')
    expect(pill(wrapper, 'All').attributes('aria-pressed')).toBe('false')
    expect(titles(wrapper)).toEqual(['Nightly'])

    await pill(wrapper, 'Healthy').trigger('click')
    expect(titles(wrapper)).toEqual(['Hourly'])
  })

  it('opens a failed run in the Activity log from its last-run outcome, not the schedule', async () => {
    const { wrapper, push } = mountTable()
    const outcome = wrapper.get('[data-schedule-id="1"] .sched-last-link')
    expect(outcome.text()).toBe('Failed')
    expect(outcome.classes()).toContain('text-danger')

    await outcome.trigger('click')

    expect(push).toHaveBeenCalledWith('/activity?category=backup&schedule_id=1&status=failed')
    // The click stays on the outcome rather than opening the row.
    expect(wrapper.emitted('open')).toBeUndefined()
  })
})
