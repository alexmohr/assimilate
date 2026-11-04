// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'

vi.mock('../composables/useTimezone', () => ({
  getConfiguredTimezone: (): string | undefined => undefined,
}))

import { renderWithPlugins } from '../test-utils'
import { repoFixture } from '../test-utils/repoFixtures'
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

describe('RepoListRow', () => {
  it('links to the repository and names it over its address and settings', () => {
    const wrapper = mount()
    expect(wrapper.find('a.list-row').attributes('href')).toBe('/repos/12')
    expect(wrapper.find('.list-row-name').text()).toBe('server-daily')
    expect(wrapper.find('.list-row-sub').text()).toBe(
      'borg@backup.example.com:/backup/repos/server-daily · repokey-blake2 · zstd,6',
    )
  })

  it('shows one status badge and a matching dot', () => {
    const wrapper = mount()
    const badges = wrapper.findAll('.badge')
    expect(badges).toHaveLength(1)
    expect(badges[0]!.text()).toBe('Healthy')
    expect(badges[0]!.classes()).toContain('badge--success')
    expect(wrapper.find('.status-dot').classes()).toContain('status-dot--success')
  })

  it('leads a row with unmatched archives to its Archives tab', () => {
    const wrapper = mount({ unmatched_count: 2 })
    expect(wrapper.find('a.list-row').attributes('href')).toBe('/repos/12?tab=archives')
    expect(wrapper.find('.badge').text()).toBe('2 unmatched')
    expect(wrapper.find('.repo-row-status').classes()).toContain('repo-row-status--warning')
  })

  it('shows the archive count and the last backup', () => {
    const wrapper = mount({ archive_count: 1, last_backup_at: null })
    const values = wrapper.findAll('.list-row-value').map((v) => v.text())
    expect(values).toEqual(['1', 'Never'])
    expect(wrapper.find('.repo-row-counts').text()).toContain('1 archive ·')
    expect(wrapper.find('.repo-row-when').text()).toBe('Never')
  })

  it('shows just the size when the repository has no quota', () => {
    const wrapper = mount({ total_deduplicated_size: 1024 })
    expect(wrapper.find('.repo-row-quota .progress-track').exists()).toBe(false)
    expect(wrapper.find('.repo-row-size').text()).toBe('1.0 KB')
    expect(wrapper.find('.repo-row-counts').text()).toBe('30 archives · 1.0 KB')
  })

  it('draws a quota bar with the size of the quota', () => {
    const wrapper = mount({ quota: QUOTA, total_deduplicated_size: 250 })
    const bar = wrapper.find('.repo-row-quota .progress-bar')
    expect(bar.attributes('style')).toContain('width: 25%')
    expect(bar.classes()).not.toContain('progress-bar--warning')
    expect(wrapper.find('.repo-row-size').text()).toBe('250.0 B of 1000.0 B')
  })

  it('tones the bar and the status by the quota health', () => {
    const near = mount({ quota: QUOTA, total_deduplicated_size: 600 })
    expect(near.find('.progress-bar').classes()).toContain('progress-bar--warning')
    expect(near.find('.badge').text()).toBe('Near quota')

    const over = mount({ quota: QUOTA, total_deduplicated_size: 5000 })
    expect(over.find('.progress-bar').classes()).toContain('progress-bar--danger')
    expect(over.find('.progress-bar').attributes('style')).toContain('width: 100%')
    expect(over.find('.badge').text()).toBe('Over quota')
    expect(over.find('.badge').attributes('title')).toBe('Block backups')
  })

  it('mutes the bar of a disabled repository', () => {
    const wrapper = mount({ enabled: false, quota: QUOTA, total_deduplicated_size: 100 })
    expect(wrapper.find('.badge').text()).toBe('Disabled')
    expect(wrapper.find('.progress-bar').classes()).toContain('progress-bar--muted')
  })

  it('pulses the badge while importing and explains a failed import', () => {
    const importing = mount({ importing: true, import_total: 0 })
    expect(importing.find('.badge').classes()).toContain('badge--pulse')

    const failed = mount({ import_error: 'boom' })
    expect(failed.find('.badge').text()).toBe('Import failed')
    expect(failed.find('.badge').attributes('title')).toBe('boom')
    expect(failed.find('.repo-row-status').text()).toBe('Import failed')
  })

  it('notes a sleeping host unless the row sits under its host header', () => {
    const sleeping = { repo_host: { id: 5, machine_id: 20, intermittent: true } }
    expect(mount(sleeping).find('.list-row-sub').text()).toContain(' · host sleeps')
    expect(mount(sleeping, { pooled: true }).find('.list-row-sub').text()).not.toContain(
      'host sleeps',
    )
    expect(mount().find('.list-row-sub').text()).not.toContain('host sleeps')
  })

  it('dims the row on request', () => {
    expect(mount({}, { dim: true }).find('a').classes()).toContain('repo-row--dim')
    expect(mount().find('a').classes()).not.toContain('repo-row--dim')
  })
})
