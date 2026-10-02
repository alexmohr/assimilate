// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'

vi.mock('../composables/useTimezone', () => ({
  getConfiguredTimezone: (): string | undefined => undefined,
}))

import { renderWithPlugins } from '../test-utils'
import { repoFixture } from '../test-utils/repoFixtures'
import RepoCard from './RepoCard.vue'
import type { EntityIssue } from './EntityStatusBadges.vue'
import type { RepoWithStats } from '../types/repo'

function mount(
  repo: Partial<RepoWithStats> = {},
  props: Record<string, unknown> = {},
  slots: Record<string, string> = {},
) {
  return renderWithPlugins(RepoCard, {
    props: { repo: repoFixture(repo), issues: [], ...props },
    slots,
  })
}

describe('RepoCard', () => {
  it('renders the name, the SSH address and the repository settings', () => {
    const wrapper = mount()
    expect(wrapper.find('.card-name').text()).toBe('server-daily')
    expect(wrapper.find('.card-ssh').text()).toBe('borg@backup.example.com:22')
    expect(wrapper.findAll('.meta-pill').map((p) => p.text())).toEqual(['repokey-blake2', 'zstd,6'])
  })

  it('shows archives, deduplicated size and last backup on a full card', () => {
    const labels = mount()
      .findAll('.stat-label')
      .map((l) => l.text())
    expect(labels).toEqual(['Archives', 'Deduplicated', 'Last backup'])
  })

  it('marks a disabled repository as notable', () => {
    const wrapper = mount({ enabled: false })
    expect(wrapper.classes()).toContain('entity-card--notable')
    expect(wrapper.text()).toContain('Disabled')
  })

  it('dims the card on request', () => {
    expect(mount({}, { dim: true }).classes()).toContain('entity-card--dim')
    expect(mount().classes()).not.toContain('entity-card--dim')
  })

  it('emits select when clicked', async () => {
    const wrapper = mount()
    await wrapper.trigger('click')
    expect(wrapper.emitted('select')).toHaveLength(1)
  })

  it('renders the tags and the host-sleeps pill', () => {
    const wrapper = mount({ repo_host: { id: 5, intermittent: true } } as Partial<RepoWithStats>, {
      tags: [{ name: 'prod', color: '#ff0000' }],
    })
    expect(wrapper.text()).toContain('host sleeps')
    const tag = wrapper.find('.tag-pill')
    expect(tag.text()).toBe('prod')
    expect(tag.attributes('style')).toContain('color: #ff0000')
  })

  it('flags a failed import', () => {
    const badge = mount({ import_error: 'boom' }).find('.badge--danger')
    expect(badge.text()).toBe('Import Failed')
    expect(badge.attributes('title')).toBe('boom')
  })

  it('shows import progress with the phase from the status message', () => {
    const wrapper = mount({
      importing: true,
      import_progress: 1,
      import_total: 4,
      import_status_message: 'Indexing archive 1 of 4',
    })
    expect(wrapper.find('.badge--warning').text()).toBe('Indexing 1/4')
    expect(wrapper.find('.progress-label').text()).toBe('25%')
    expect(wrapper.find('.import-status-inline').text()).toBe('Indexing archive 1 of 4')
  })

  it('shows an open-ended import before the total is known', () => {
    const wrapper = mount({ importing: true, import_total: 0 })
    expect(wrapper.find('.badge--warning').text()).toBe('Importing\u2026')
    expect(wrapper.find('.progress-row').exists()).toBe(false)
  })

  it('renders the issues it is given', () => {
    const issues: EntityIssue[] = [
      { key: 'unmatched', label: '2 unmatched', severity: 'warning', onClick: () => {} },
    ]
    expect(mount({}, { issues }).text()).toContain('2 unmatched')
  })

  it('trims a pooled card to what the pool header does not already say', () => {
    const wrapper = mount(
      {
        importing: true,
        import_progress: 1,
        import_total: 4,
        import_status_message: 'Importing',
        repo_host: { id: 5, intermittent: true },
      } as Partial<RepoWithStats>,
      { pooled: true, tags: [{ name: 'prod', color: '#ff0000' }] },
    )
    expect(wrapper.find('.progress-row').exists()).toBe(false)
    expect(wrapper.find('.import-status-inline').exists()).toBe(false)
    expect(wrapper.find('.tag-pill').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('host sleeps')
    expect(wrapper.findAll('.stat-label').map((l) => l.text())).toEqual(['Archives', 'Last backup'])
  })

  it('lets the caller replace the quota meter', () => {
    const wrapper = mount({}, {}, { quota: '<div class="custom-quota">slice</div>' })
    expect(wrapper.find('.custom-quota').exists()).toBe(true)
  })
})
