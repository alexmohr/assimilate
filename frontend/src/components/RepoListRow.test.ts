// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'

vi.mock('../composables/useTimezone', () => ({
  getConfiguredTimezone: (): string | undefined => undefined,
}))

import { renderWithPlugins } from '../test-utils'
import { appendOnly, repoFixture } from '../test-utils/repoFixtures'
import RepoListRow from './RepoListRow.vue'
import type { RepoWithStats } from '../types/repo'

type Quota = NonNullable<RepoWithStats['quota']>

const QUOTA: Quota = {
  warn_bytes: 500,
  critical_bytes: 1000,
  warn_action: 'notify_only',
  critical_action: 'block_backups',
  enabled: true,
}

function mount(repo: Partial<RepoWithStats> = {}, props: Record<string, unknown> = {}) {
  return renderWithPlugins(RepoListRow, {
    props: { repo: repoFixture({ unmatched_count: 0, quota: null, ...repo }), ...props },
  })
}

type Wrapper = ReturnType<typeof mount>

/** The facts under the name, one entry per fact. */
function facts(wrapper: Wrapper): string[] {
  return wrapper.findAll('.repo-row-facts > span').map((s) => s.text())
}

/** The status named first in the facts; absent for a healthy repository. */
function statusFact(wrapper: Wrapper) {
  return wrapper.findAll('.repo-row-facts > span')[0]!
}

describe('RepoListRow', () => {
  it('links to the repository and names it over its facts', () => {
    const wrapper = mount()
    expect(wrapper.find('a.list-row').attributes('href')).toBe('/repos/12')
    expect(wrapper.find('.list-row-name').text()).toBe('server-daily')
    expect(facts(wrapper)).toHaveLength(2)
    expect(facts(wrapper)[0]).toBe('30 archives')
    expect(facts(wrapper)[1]).toMatch(/^last backup /)
  })

  it('shows a healthy repository by its dot alone', () => {
    const wrapper = mount()
    expect(wrapper.findAll('.badge')).toHaveLength(0)
    expect(wrapper.find('.status-dot').classes()).toContain('status-dot--success')
    expect(wrapper.text()).not.toContain('Healthy')
  })

  it('leads a row with unmatched archives to its Archives tab without flagging a risk', () => {
    const wrapper = mount({ unmatched_count: 2 })
    expect(wrapper.find('a.list-row').attributes('href')).toBe('/repos/12?tab=archives')
    const status = statusFact(wrapper)
    expect(status.text()).toBe('2 unmatched')
    expect(status.attributes('title')).toBe('Imported archives no schedule claims yet')
    // Unmatched archives are not a risk: the dot stays green and the label muted.
    expect(status.classes()).not.toContain('repo-row-status--warning')
    expect(wrapper.find('.status-dot').classes()).toContain('status-dot--success')
  })

  it('shows the archive count and the last backup', () => {
    const one = mount({ archive_count: 1, last_backup_at: null })
    expect(facts(one)).toEqual(['1 archive', 'no backup yet'])
  })

  it('names append-only, a stopped maintenance and queued deletions among the facts', () => {
    expect(facts(mount())).toEqual(['30 archives', expect.stringMatching(/^last backup /)])

    const on = mount({ append_only: appendOnly() })
    expect(facts(on)).toEqual([
      'append-only',
      '30 archives',
      expect.stringMatching(/^last backup /),
    ])
    // Not a risk, so it stays in the muted style and the dot stays as it was.
    expect(statusFact(on).classes()).toEqual([])
    expect(on.find('.status-dot').classes()).toContain('status-dot--success')

    const stopped = mount({
      append_only: appendOnly({ maintenance_stopped: true, pending_deletion_count: 3 }),
    })
    expect(facts(stopped).slice(0, 3)).toEqual([
      'append-only',
      'maintenance stopped',
      '3 pending deletion',
    ])
    const stoppedFact = stopped.findAll('.repo-row-facts > span')[1]!
    expect(stoppedFact.classes()).toContain('repo-row-status--warning')
  })

  it('shows just the size when the repository has no quota', () => {
    const wrapper = mount({ total_deduplicated_size: 1024 })
    expect(wrapper.find('.repo-row-quota .progress-track').exists()).toBe(false)
    expect(wrapper.find('.repo-row-size').text()).toBe('1.0 KB')
  })

  it('draws a quota bar with the size of the quota', () => {
    const wrapper = mount({ quota: QUOTA, total_deduplicated_size: 250 })
    const bar = wrapper.find('.repo-row-quota .progress-bar')
    expect(bar.attributes('style')).toContain('width: 25%')
    expect(bar.classes()).not.toContain('progress-bar--warning')
    expect(wrapper.find('.repo-row-size').text()).toBe('250.0 B of 1000.0 B')
  })

  it('tones the bar, the dot and the status by the quota health', () => {
    const near = mount({ quota: QUOTA, total_deduplicated_size: 600 })
    expect(near.find('.progress-bar').classes()).toContain('progress-bar--warning')
    expect(statusFact(near).text()).toBe('Near quota')
    expect(statusFact(near).classes()).toContain('repo-row-status--warning')
    expect(near.find('.status-dot').classes()).toContain('status-dot--warning')

    const over = mount({ quota: QUOTA, total_deduplicated_size: 5000 })
    expect(over.find('.progress-bar').classes()).toContain('progress-bar--danger')
    expect(over.find('.progress-bar').attributes('style')).toContain('width: 100%')
    expect(statusFact(over).text()).toBe('Over quota')
    expect(statusFact(over).attributes('title')).toBe('Block backups')
    expect(statusFact(over).classes()).toContain('repo-row-status--danger')
    expect(over.find('.status-dot').classes()).toContain('status-dot--danger')
  })

  it('mutes the bar of a disabled repository', () => {
    const wrapper = mount({ enabled: false, quota: QUOTA, total_deduplicated_size: 100 })
    expect(statusFact(wrapper).text()).toBe('Disabled')
    expect(statusFact(wrapper).classes()).toEqual([])
    expect(wrapper.find('.progress-bar').classes()).toContain('progress-bar--muted')
    // A neutral status has no tinted dot.
    expect(wrapper.find('.status-dot').classes()).toEqual(['status-dot'])
  })

  it('names an import in progress and explains a failed import', () => {
    const importing = mount({ importing: true, import_total: 0 })
    expect(statusFact(importing).text()).toBe('Importing…')
    expect(importing.find('.status-dot').classes()).toContain('status-dot--warning')

    const failed = mount({ import_error: 'boom' })
    expect(statusFact(failed).text()).toBe('Import failed')
    expect(statusFact(failed).attributes('title')).toBe('boom')
    expect(statusFact(failed).classes()).toContain('repo-row-status--danger')
  })

  it('dims the row on request', () => {
    expect(mount({}, { dim: true }).find('a').classes()).toContain('repo-row--dim')
    expect(mount().find('a').classes()).not.toContain('repo-row--dim')
  })
})
