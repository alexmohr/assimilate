// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import ScheduleBackupsTab from './ScheduleBackupsTab.vue'
import type { ReportRow } from '../types/report'
import type { AgentRow } from '../types/agent'

const AGENTS = new Map<number, AgentRow>([
  [10, { id: 10, hostname: 'web-01', display_name: 'Web 01' } as unknown as AgentRow],
])

function report(overrides: Partial<ReportRow>): ReportRow {
  return {
    id: 1,
    status: 'success',
    archive_name: 'web-01-2026-01-01',
    started_at: '2026-01-01T02:00:00Z',
    original_size: 1024,
    deduplicated_size: 512,
    agent_id: 10,
    hostname: 'web-01',
    // Reports carry the repository they were written to; the tab browses one.
    repo_id: 3,
    ...overrides,
  } as unknown as ReportRow
}

function mount(props: Record<string, unknown> = {}) {
  const reports = (props.reports as unknown[] | undefined) ?? [report({})]
  return renderWithPlugins(ScheduleBackupsTab, {
    props: {
      reports,
      total: reports.length,
      loading: false,
      loadingMore: false,
      error: null,
      agents: AGENTS,
      repoId: 3,
      selected: null,
      ...props,
    },
  })
}

describe('ScheduleBackupsTab', () => {
  /** The reports are the whole schedule's, so a multi-target schedule's list
      includes runs against every target - but this tab browses and deletes
      against one `repoId`. Listing another target's archive would send its
      delete to the wrong repository. */
  it('leaves out archives written to a different target repository', () => {
    const wrapper = mount({
      reports: [
        report({ id: 1, archive_name: 'on-primary' }),
        report({ id: 2, archive_name: 'on-offsite', repo_id: 4 }),
      ],
    })

    expect(wrapper.text()).toContain('on-primary')
    expect(wrapper.text()).not.toContain('on-offsite')
  })

  it('shows placeholder rows while loading', () => {
    expect(mount({ loading: true }).find('.archive-loading').exists()).toBe(true)
  })

  it('shows the error instead of the browser', () => {
    expect(mount({ error: 'boom' }).find('.error-banner').text()).toBe('boom')
  })

  it('lists only runs that produced an archive', () => {
    const wrapper = mount({
      reports: [
        report({ id: 1, archive_name: 'kept' }),
        report({ id: 2, archive_name: null }),
        report({ id: 3, archive_name: 'failed-run', status: 'failed' }),
      ],
    })
    const names = wrapper.findAll('.archive-name').map((c) => c.text())
    expect(names).toEqual(['kept'])
  })

  it('keeps warning runs, whose archive exists but is incomplete', () => {
    const wrapper = mount({
      reports: [report({ id: 2, archive_name: 'partial', status: 'warning' })],
    })
    expect(wrapper.findAll('.archive-name').map((c) => c.text())).toEqual(['partial'])
  })

  it('orders the archives newest first', () => {
    const wrapper = mount({
      reports: [
        report({ id: 1, archive_name: 'jan', started_at: '2026-01-01T00:00:00Z' }),
        report({ id: 2, archive_name: 'mar', started_at: '2026-03-01T00:00:00Z' }),
        report({ id: 3, archive_name: 'feb', started_at: '2026-02-01T00:00:00Z' }),
      ],
    })
    expect(wrapper.findAll('.archive-name').map((c) => c.text())).toEqual(['mar', 'feb', 'jan'])
  })

  it('says so when the schedule has produced no archives', () => {
    const wrapper = mount({ reports: [report({ archive_name: null })] })
    expect(wrapper.find('.empty-state').text()).toContain('No backup archives found')
  })

  it('reports the clicked archive up to the view, which owns the selection', async () => {
    const wrapper = mount()
    await wrapper.find('.archive-row-select').trigger('click')
    const updates = wrapper.emitted('update:selected')
    expect(updates).toHaveLength(1)
    expect((updates![0][0] as ReportRow).archive_name).toBe('web-01-2026-01-01')
  })

  it('offers deletion to an admin and withholds it from everyone else', () => {
    // The schedule's own table had no delete at all; it now shares the
    // repository screen's control, gated the same way.
    expect(mount({ isAdmin: true }).find('.archive-row-delete').exists()).toBe(true)
    expect(mount({ isAdmin: false }).find('.archive-row-delete').exists()).toBe(false)
  })

  it('groups the archives by host, like every other archive screen', () => {
    const wrapper = mount({
      reports: [
        report({ id: 1, archive_name: 'web', agent_id: 10 }),
        report({ id: 2, archive_name: 'db', agent_id: 11, hostname: 'db-01' }),
      ],
    })
    expect(wrapper.findAll('.group-hostname').map((g) => g.text())).toEqual(['db-01', 'web-01'])
  })

  it('still renders for a caller that passes no reload', () => {
    // `reload` is optional; the tab falls back to a resolved promise so the
    // explorer's post-delete refresh has something to await either way.
    const wrapper = mount({ reload: undefined })
    expect(wrapper.find('.archive-row').exists()).toBe(true)
  })

  it('marks the selected row', () => {
    const wrapper = mount({ selected: report({}) })
    expect(wrapper.find('.archive-row').classes()).toContain('selected')
  })

  it('leaves every row unmarked when nothing is selected', () => {
    expect(mount().find('.archive-row').classes()).not.toContain('selected')
  })

  // Regression test: unlike `AgentArchivesTab`, this tab derives its archive
  // list from a capped, paged report fetch shared with the Logs tab (there is
  // no per-schedule uncapped archive endpoint to read from instead) - so
  // without this, a schedule with more history than has been loaded would
  // silently show fewer archives than exist, with nothing on screen to say so.
  it('offers to load older runs when the loaded page is not the whole history', () => {
    const wrapper = mount({ reports: [report({})], total: 75 })
    expect(wrapper.text()).toContain('Load 50 more runs')
    expect(wrapper.text()).toContain("Only this schedule's 1 most recent runs (of 75)")
  })

  it('hides the load-more affordance once every report is loaded', () => {
    const wrapper = mount({ reports: [report({})], total: 1 })
    expect(wrapper.text()).not.toContain('more runs')
  })

  it('emits loadMore when the load-more button is clicked', async () => {
    const wrapper = mount({ reports: [report({})], total: 75 })
    await wrapper.find('.backups-more-row button').trigger('click')
    expect(wrapper.emitted('loadMore')).toHaveLength(1)
  })

  it('disables the load-more button while a page is already loading', () => {
    const wrapper = mount({ reports: [report({})], total: 75, loadingMore: true })
    const button = wrapper.find('.backups-more-row button')
    expect(button.attributes('disabled')).toBeDefined()
    expect(button.text()).toBe('Loading...')
  })
})
