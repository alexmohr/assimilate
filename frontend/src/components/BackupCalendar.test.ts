// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { flushPromises, type VueWrapper } from '@vue/test-utils'
import type { ComponentPublicInstance } from 'vue'
import { renderWithPlugins } from '../test-utils'
import { SRC } from '../test-utils/vueFiles'
import { apiClient } from '../api/client'
import type { CalendarDayResponse, CalendarEventResponse } from '../types/generated'
import BackupCalendar from './BackupCalendar.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn().mockResolvedValue({ data: [] }),
  },
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn(), warn: vi.fn(), info: vi.fn() },
}))

describe('BackupCalendar', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders without throwing', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: { repos: [] },
    })
    expect(wrapper.exists()).toBe(true)
  })

  it('shows loading state initially', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: { repos: [] },
    })
    expect(wrapper.text()).toContain('Loading')
  })

  it('displays the panel title', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: { repos: [] },
    })
    expect(wrapper.text()).toContain('Backup calendar')
  })

  it('renders repo options in select', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: {
        repos: [
          { id: 1, name: 'daily-backups' },
          { id: 2, name: 'weekly-archive' },
        ],
      },
    })
    expect(wrapper.text()).toContain('daily-backups')
    expect(wrapper.text()).toContain('weekly-archive')
  })

  it('sizes the seven day columns so none can be clipped away', () => {
    // A plain `repeat(7, 1fr)` track will not shrink below its content's
    // min-content width, so a busy day widened the whole grid past the panel
    // and the clip took Saturday with it. `minmax(0, 1fr)` lets the columns
    // fit the panel and confines any overflow to the cell that caused it.
    const css = readFileSync(join(SRC, 'components', 'BackupCalendar.vue'), 'utf-8')
    expect(css).toContain('grid-template-columns: repeat(7, minmax(0, 1fr))')
    expect(css).not.toMatch(/grid-template-columns:\s*repeat\(7,\s*1fr\)/)
  })

  it('renders navigation buttons for month switching', () => {
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: { repos: [] },
    })
    const buttons = wrapper.findAll('button')
    expect(buttons.length).toBeGreaterThanOrEqual(2)
  })
})

function calendarEvent(overrides: Partial<CalendarEventResponse>): CalendarEventResponse {
  return {
    type: 'backup',
    status: 'success',
    repo_name: 'daily-backups',
    hostname: 'web-1',
    time: '02:00',
    report_id: null,
    repo_id: null,
    schedule_id: null,
    archive_name: null,
    error_message: null,
    ...overrides,
  }
}

type Wrapper = VueWrapper<ComponentPublicInstance>

/** Renders the calendar with `days` as the API's answer and waits for it. */
async function renderCalendar(days: CalendarDayResponse[] = []): Promise<Wrapper> {
  vi.mocked(apiClient.get).mockResolvedValue({ data: days })
  const wrapper = renderWithPlugins(BackupCalendar, { props: { repos: [] } })
  await flushPromises()
  return wrapper
}

function lastRequestedUrl(): string {
  const calls = vi.mocked(apiClient.get).mock.calls
  return String(calls[calls.length - 1][0])
}

function dayCell(wrapper: Wrapper, day: number): ReturnType<Wrapper['find']> {
  const cell = wrapper
    .findAll('.cal-cell-active')
    .find((c) => c.find('.cal-day-num').text() === String(day))
  if (!cell) throw new Error(`no cell for day ${day}`)
  return cell
}

async function openDay(wrapper: Wrapper, day: number): Promise<void> {
  await dayCell(wrapper, day).trigger('click')
}

async function clickEvent(wrapper: Wrapper, index: number): Promise<void> {
  await wrapper.findAll('.cal-event')[index].trigger('click')
  await flushPromises()
}

async function clickAndSettle(target: ReturnType<Wrapper['find']>): Promise<void> {
  await target.trigger('click')
  await flushPromises()
}

function currentPath(wrapper: Wrapper): string {
  return wrapper.vm.$router.currentRoute.value.fullPath
}

describe('BackupCalendar month grid and events', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    // Only the clock is faked: the grid depends on which weekday the month
    // starts on, so a real "today" would cover the padding cells on some
    // days of the year and not on others.
    vi.useFakeTimers({ toFake: ['Date'] })
    vi.setSystemTime(new Date(2026, 8, 15, 12))
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('pads a month on both sides to whole weeks', async () => {
    // September 2026 starts on a Tuesday and has 30 days: two blank cells
    // before the 1st and three after the 30th make five full weeks.
    const wrapper = await renderCalendar()
    expect(lastRequestedUrl()).toBe('/stats/calendar?month=2026-09')
    const inMonth = wrapper.findAll('.cal-cell').map((c) => c.classes('cal-cell-active'))
    expect(inMonth).toHaveLength(35)
    expect(inMonth.slice(0, 2)).toEqual([false, false])
    expect(inMonth.slice(2, 32).every(Boolean)).toBe(true)
    expect(inMonth.slice(32)).toEqual([false, false, false])
  })

  it('adds no padding to a month that fills whole weeks exactly', async () => {
    // February 2026 starts on a Sunday and has 28 days.
    vi.setSystemTime(new Date(2026, 1, 10, 12))
    const wrapper = await renderCalendar()
    expect(wrapper.findAll('.cal-cell')).toHaveLength(28)
    expect(wrapper.findAll('.cal-cell-active')).toHaveLength(28)
  })

  it('steps back across a year boundary and forward again', async () => {
    vi.setSystemTime(new Date(2026, 0, 10, 12))
    const wrapper = await renderCalendar()
    const [prev, next] = wrapper.findAll('.cal-nav-btn')

    await clickAndSettle(prev)
    expect(lastRequestedUrl()).toBe('/stats/calendar?month=2025-12')
    await clickAndSettle(next)
    expect(lastRequestedUrl()).toBe('/stats/calendar?month=2026-01')
  })

  it('steps forward across a year boundary', async () => {
    vi.setSystemTime(new Date(2026, 11, 10, 12))
    const wrapper = await renderCalendar()
    await clickAndSettle(wrapper.findAll('.cal-nav-btn')[1])
    expect(lastRequestedUrl()).toBe('/stats/calendar?month=2027-01')
  })

  it('refetches for the repository picked in the filter', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(BackupCalendar, {
      props: { repos: [{ id: 7, name: 'offsite' }] },
    })
    await flushPromises()
    await wrapper.find('select').setValue('7')
    await flushPromises()
    expect(lastRequestedUrl()).toBe('/stats/calendar?month=2026-09&repo_id=7')
  })

  it('colours up to four dots per day by status and counts the rest', async () => {
    const statuses = ['success', 'failed', 'warning', 'scheduled', 'running']
    const wrapper = await renderCalendar([
      { date: '2026-09-03', events: statuses.map((status) => calendarEvent({ status })) },
    ])
    const cell = dayCell(wrapper, 3)
    expect(cell.classes()).toContain('cal-cell-has-events')
    expect(cell.findAll('.cal-dot').map((d) => d.attributes('style'))).toEqual([
      'background: var(--success);',
      'background: var(--danger);',
      'background: var(--warning);',
      'background: var(--info);',
    ])
    expect(cell.find('.cal-dot-more').text()).toBe('+1')
  })

  it('toggles a day open and closed, and ignores days without events', async () => {
    const wrapper = await renderCalendar([
      { date: '2026-09-03', events: [calendarEvent({ status: 'running' })] },
    ])

    await openDay(wrapper, 3)
    expect(wrapper.find('.cal-detail-title').text()).toBe('2026-09-03')
    expect(dayCell(wrapper, 3).classes()).toContain('cal-cell-selected')
    // A status that leads nowhere is listed but not clickable.
    expect(wrapper.find('.cal-event').classes()).not.toContain('cal-event-clickable')

    await openDay(wrapper, 3)
    expect(wrapper.find('.cal-detail').exists()).toBe(false)

    await openDay(wrapper, 3)
    await openDay(wrapper, 4)
    expect(wrapper.find('.cal-detail').exists()).toBe(false)
  })

  it('closes the open day when the month changes', async () => {
    const wrapper = await renderCalendar([{ date: '2026-09-03', events: [calendarEvent({})] }])
    await openDay(wrapper, 3)
    expect(wrapper.find('.cal-detail').exists()).toBe(true)

    await clickAndSettle(wrapper.findAll('.cal-nav-btn')[1])
    expect(wrapper.find('.cal-detail').exists()).toBe(false)
  })

  it('opens the archive of a successful run on its repository', async () => {
    const wrapper = await renderCalendar([
      {
        date: '2026-09-03',
        events: [
          calendarEvent({ repo_id: 4, archive_name: 'web-1-2026-09-03' }),
          calendarEvent({ repo_id: 5 }),
        ],
      },
    ])
    await openDay(wrapper, 3)

    await clickEvent(wrapper, 0)
    expect(currentPath(wrapper)).toBe('/repos/4?tab=archives&archive=web-1-2026-09-03')
    await clickEvent(wrapper, 1)
    expect(currentPath(wrapper)).toBe('/repos/5?tab=archives')
  })

  it('opens the schedule of a planned run', async () => {
    const wrapper = await renderCalendar([
      { date: '2026-09-03', events: [calendarEvent({ status: 'scheduled', schedule_id: 9 })] },
    ])
    await openDay(wrapper, 3)
    await clickEvent(wrapper, 0)
    expect(currentPath(wrapper)).toBe('/schedules/9')
  })

  it('shows why a run failed and links to its repository', async () => {
    const wrapper = await renderCalendar([
      {
        date: '2026-09-03',
        events: [
          calendarEvent({
            status: 'failed',
            repo_id: 4,
            schedule_id: 9,
            error_message: 'Repository is locked',
          }),
        ],
      },
    ])
    await openDay(wrapper, 3)
    await clickEvent(wrapper, 0)

    expect(wrapper.find('.cal-error-msg').text()).toBe('Repository is locked')
    const links = wrapper.findAll('.cal-error-link')
    expect(links.map((l) => l.text())).toEqual(['daily-backups', 'Schedule'])

    await clickAndSettle(links[0])
    expect(currentPath(wrapper)).toBe('/repos/4')
    expect(wrapper.find('.cal-error-popup').exists()).toBe(false)
  })

  it('links a failure popup to its schedule', async () => {
    const wrapper = await renderCalendar([
      {
        date: '2026-09-03',
        events: [calendarEvent({ status: 'failed', repo_id: 4, schedule_id: 9 })],
      },
    ])
    await openDay(wrapper, 3)
    await clickEvent(wrapper, 0)
    expect(wrapper.find('.cal-error-msg').text()).toBe('No error details available.')

    await clickAndSettle(wrapper.findAll('.cal-error-link')[1])
    expect(currentPath(wrapper)).toBe('/schedules/9')
    expect(wrapper.find('.cal-error-popup').exists()).toBe(false)
  })

  it('explains a warning and closes from the close button or the backdrop', async () => {
    const wrapper = await renderCalendar([
      {
        date: '2026-09-03',
        events: [
          calendarEvent({ status: 'warning', error_message: 'file changed while reading' }),
          calendarEvent({ status: 'warning' }),
        ],
      },
    ])
    await openDay(wrapper, 3)

    await clickEvent(wrapper, 0)
    expect(wrapper.find('.cal-error-msg').text()).toBe('file changed while reading')
    // Without a repository or schedule there is nothing to link to.
    expect(wrapper.findAll('.cal-error-link')).toHaveLength(0)
    await wrapper.find('.cal-error-close').trigger('click')
    expect(wrapper.find('.cal-error-popup').exists()).toBe(false)

    await clickEvent(wrapper, 1)
    expect(wrapper.find('.cal-error-msg').text()).toBe('No warning details available.')
    await wrapper.find('.cal-error-overlay').trigger('click')
    expect(wrapper.find('.cal-error-popup').exists()).toBe(false)
  })

  it('opens the repository from an event row without opening the event', async () => {
    const wrapper = await renderCalendar([
      { date: '2026-09-03', events: [calendarEvent({ status: 'failed', repo_id: 4 })] },
    ])
    await openDay(wrapper, 3)
    await clickAndSettle(wrapper.find('.cal-event-repo-link'))
    expect(currentPath(wrapper)).toBe('/repos/4')
    expect(wrapper.find('.cal-error-popup').exists()).toBe(false)
  })
})
