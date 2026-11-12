// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'
import { renderWithPlugins } from '../test-utils'

// The open archive's file tree fetches from the server and carries the
// archive's menu, Delete among it; this tab only decides what it is handed.
vi.mock('./ArchiveFileBrowser.vue', () => ({
  default: {
    name: 'ArchiveFileBrowser',
    props: ['repoId', 'repoName', 'archive', 'archives', 'isAdmin', 'deleting'],
    emits: ['delete-archive'],
    template:
      '<div class="stub-browser"><button v-if="isAdmin" class="stub-delete" type="button">Delete archive...</button></div>',
  },
}))

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
      scheduleId: '1',
      repoId: 3,
      selected: null,
      ...props,
    },
  })
}

type Wrapper = ReturnType<typeof mount>

/** Every archive row, in the order the list shows them. */
function archiveRows(wrapper: Wrapper) {
  return wrapper.findAll('[data-testid="archive-row"]')
}

/** The archive names, in list order. */
function names(wrapper: Wrapper): string[] {
  return archiveRows(wrapper).map((r) => r.get('.browser-row-title').text())
}

/** The row marked as the open archive, by `aria-current`. */
function selectedRows(wrapper: Wrapper) {
  return archiveRows(wrapper).filter((r) => r.attributes('aria-current') === 'true')
}

/** The repository filter's pills, present only for a schedule with several targets. */
function repoPills(wrapper: Wrapper) {
  return wrapper.findAll('[role="group"][aria-label="Repository"] .filter-pill')
}

const REPOS = [
  { id: 3, name: 'server-daily', required: true },
  { id: 4, name: 'offsite-weekly', required: false },
]

describe('ScheduleBackupsTab', () => {
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
    expect(names(wrapper)).toEqual(['kept'])
  })

  it('keeps warning runs, whose archive exists but is incomplete', () => {
    const wrapper = mount({
      reports: [report({ id: 2, archive_name: 'partial', status: 'warning' })],
    })
    expect(names(wrapper)).toEqual(['partial'])
  })

  it('orders the archives newest first', () => {
    const wrapper = mount({
      reports: [
        report({ id: 1, archive_name: 'jan', started_at: '2026-01-01T00:00:00Z' }),
        report({ id: 2, archive_name: 'mar', started_at: '2026-03-01T00:00:00Z' }),
        report({ id: 3, archive_name: 'feb', started_at: '2026-02-01T00:00:00Z' }),
      ],
    })
    expect(names(wrapper)).toEqual(['mar', 'feb', 'jan'])
  })

  it('says so when the schedule has produced no archives', () => {
    const wrapper = mount({ reports: [report({ archive_name: null })] })
    expect(wrapper.find('.empty-state').text()).toContain('No backup archives found')
  })

  it('reports the clicked archive up to the view, which owns the selection', async () => {
    const wrapper = mount()
    await archiveRows(wrapper)[0]!.trigger('click')
    const updates = wrapper.emitted('update:selected')
    expect(updates).toHaveLength(1)
    expect((updates![0][0] as ReportRow).archive_name).toBe('web-01-2026-01-01')
  })

  it('offers deletion to an admin and withholds it from everyone else', () => {
    // The schedule's own table had no delete at all; it now shares the
    // repository screen's control, gated the same way: in the open archive's
    // menu rather than on each row.
    const admin = mount({ isAdmin: true, selected: report({}) })
    expect(admin.findComponent({ name: 'ArchiveFileBrowser' }).props('isAdmin')).toBe(true)
    expect(admin.find('.stub-delete').exists()).toBe(true)

    const viewer = mount({ isAdmin: false, selected: report({}) })
    expect(viewer.findComponent({ name: 'ArchiveFileBrowser' }).props('isAdmin')).toBe(false)
    expect(viewer.find('.stub-delete').exists()).toBe(false)
  })

  it('opens the selected archive in the file browser, on its own repository', () => {
    const wrapper = mount({ selected: report({}), repoOptions: [REPOS[0]] })
    const browser = wrapper.findComponent({ name: 'ArchiveFileBrowser' })
    expect(browser.props('repoId')).toBe(3)
    expect(browser.props('repoName')).toBe('server-daily')
    expect((browser.props('archive') as { name: string }).name).toBe('web-01-2026-01-01')
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
    expect(archiveRows(wrapper)).toHaveLength(1)
  })

  it('marks the selected row', () => {
    const wrapper = mount({ selected: report({}) })
    const row = archiveRows(wrapper)[0]!
    expect(row.attributes('aria-current')).toBe('true')
    expect(row.classes()).toContain('browser-row--selected')
  })

  it('leaves every row unmarked when nothing is selected', () => {
    const row = archiveRows(mount())[0]!
    expect(row.attributes('aria-current')).toBe('false')
    expect(row.classes()).not.toContain('browser-row--selected')
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
    await wrapper.find('.pager-load-more button').trigger('click')
    expect(wrapper.emitted('loadMore')).toHaveLength(1)
  })

  it('disables the load-more button while a page is already loading', () => {
    const wrapper = mount({ reports: [report({})], total: 75, loadingMore: true })
    const button = wrapper.find('.pager-load-more button')
    expect(button.attributes('disabled')).toBeDefined()
    expect(button.text()).toBe('Loading...')
  })

  // A schedule writes the same archive name into every target, and this tab
  // used to be handed the primary one alone - so the copies in every other
  // repository existed, were listed nowhere, and nothing on screen said so.
  // Then it got a drop-down that showed one target at a time. Now every
  // target's archives are one list, each row saying which copy it is.
  describe('several target repositories', () => {
    const SPLIT = [
      report({ id: 1, archive_name: 'on-primary', repo_id: 3 }),
      report({ id: 2, archive_name: 'on-offsite', repo_id: 4 }),
    ]

    function spreadMount(over: Record<string, unknown> = {}) {
      return mount({ reports: SPLIT, repoOptions: REPOS, ...over })
    }

    /** Each row's archive name and the repository it names, if any. */
    function rows(wrapper: ReturnType<typeof mount>): string[][] {
      return archiveRows(wrapper).map((r) => [r.get('.browser-row-title').text(), repoOf(r)])
    }

    /** The repository a row names among its facts; empty when it names none. */
    function repoOf(row: ReturnType<typeof archiveRows>[number]): string {
      const names = REPOS.map((repo) => repo.name)
      return (
        row
          .findAll('.browser-row-facts span')
          .map((f) => f.text())
          .find((t) => names.includes(t)) ?? ''
      )
    }

    it('lists every target in one list, each row tagged with its repository', () => {
      expect(rows(spreadMount())).toEqual([
        ['on-primary', 'server-daily'],
        ['on-offsite', 'offsite-weekly'],
      ])
    })

    it('names every target and how many archives each holds', () => {
      const options = repoPills(spreadMount()).map((o) => o.text())
      expect(options).toEqual(['All · 2', 'server-daily · 1', 'offsite-weekly · 1'])
    })

    it('offers no repository filter when the schedule writes to one repository', () => {
      const wrapper = mount({ repoOptions: [REPOS[0]] })
      expect(repoPills(wrapper)).toHaveLength(0)
      expect(rows(wrapper)).toEqual([['web-01-2026-01-01', '']])
    })

    it('narrows to one target', async () => {
      const wrapper = spreadMount()
      await repoPills(wrapper)[2]!.trigger('click')
      // Narrowed to one target, the rows stop repeating its name: the pill says it.
      expect(rows(wrapper)).toEqual([['on-offsite', '']])
      expect(repoPills(wrapper)[2]!.attributes('aria-pressed')).toBe('true')
    })

    // The same name in two targets is two copies; picking one must hand the
    // view the report behind that copy, not the first one with the name.
    it('keeps two copies of the same name apart', async () => {
      const copies = [
        report({ id: 1, archive_name: 'nightly', repo_id: 3 }),
        report({ id: 2, archive_name: 'nightly', repo_id: 4 }),
      ]
      const wrapper = mount({ reports: copies, repoOptions: REPOS })
      await archiveRows(wrapper)[1]!.trigger('click')

      expect((wrapper.emitted('update:selected')![0]![0] as ReportRow).id).toBe(2)
    })

    // The Overview's "browse this archive" hands this tab a report, and on a
    // multi-repository schedule it is as likely to belong to the secondary
    // target - which used to land on an empty pane.
    it('shows a selection made against another target, even past a filter', async () => {
      const wrapper = spreadMount()
      await repoPills(wrapper)[1]!.trigger('click')
      expect(rows(wrapper)).toEqual([['on-primary', '']])

      await wrapper.setProps({ selected: SPLIT[1] })

      const selected = selectedRows(wrapper)
      expect(selected.map((r) => r.get('.browser-row-title').text())).toEqual(['on-offsite'])
      // The filter gave way, so both targets show again.
      expect(rows(wrapper)).toHaveLength(2)
    })

    // The targets and the reports are two independent requests, so the rows
    // can arrive before the target names do.
    it('names a target once the target list arrives', async () => {
      const wrapper = mount({ reports: SPLIT, repoOptions: [] })
      await wrapper.setProps({ repoOptions: REPOS })

      expect(rows(wrapper).map(([, repo]) => repo)).toEqual(['server-daily', 'offsite-weekly'])
    })

    // The view keeps this tab mounted across `/schedules/:id` changes, so a
    // filter chosen for one schedule would otherwise carry into the next one
    // that happens to write into the same repository.
    it('forgets the repository filter when the schedule changes', async () => {
      const wrapper = spreadMount()
      await repoPills(wrapper)[2]!.trigger('click')
      expect(rows(wrapper)).toHaveLength(1)

      await wrapper.setProps({ scheduleId: '2' })

      expect(rows(wrapper)).toHaveLength(2)
    })

    // A search that hides the archive the Overview jumped to would leave the
    // file tree open on a row the list says is not there.
    it('clears a search that hides the archive the Overview jumped to', async () => {
      const wrapper = spreadMount()
      const search = wrapper.get('input[aria-label="Search archives"]')
      await search.setValue('on-primary')
      expect(rows(wrapper)).toHaveLength(1)

      await wrapper.setProps({ selected: SPLIT[1] })

      expect((search.element as HTMLInputElement).value).toBe('')
      expect(selectedRows(wrapper).map((r) => r.get('.browser-row-title').text())).toEqual([
        'on-offsite',
      ])
    })

    it('says none of the targets holds an archive yet', () => {
      const wrapper = mount({ reports: [report({ archive_name: null })], repoOptions: REPOS })
      expect(wrapper.find('.empty-state').text()).toContain(
        "No backup archives in any of this schedule's repositories yet.",
      )
    })
  })
})
