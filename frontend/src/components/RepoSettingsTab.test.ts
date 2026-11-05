// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { repoFixture } from '../test-utils/repoFixtures'
import RepoSettingsTab from './RepoSettingsTab.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn().mockResolvedValue({
      data: {
        intermittent: false,
        catch_up_recheck_minutes: 15,
        catch_up_give_up_minutes: 0,
        waiting: [],
      },
    }),
    post: vi.fn().mockRejectedValue(new Error('not reached in these tests')),
  },
}))

vi.mock('../api/appendOnly', async () => {
  const { appendOnlyStatusFixture } = await import('../test-utils/appendOnlyFixtures')
  return { getAppendOnlyStatus: vi.fn().mockResolvedValue(appendOnlyStatusFixture()) }
})

const REPO = repoFixture({
  power: {
    wake_enabled: false,
    wake_mac_address: null,
    wake_broadcast_address: null,
    wake_timeout_seconds: 180,
    shutdown_after_backup: false,
  },
} as never)

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(RepoSettingsTab, {
    props: {
      repo: REPO,
      section: 'repository',
      isAdmin: true,
      currentOp: null,
      ...props,
    },
  })
}

describe('RepoSettingsTab', () => {
  it('lists the Power & dependencies section for an admin', () => {
    expect(
      mount()
        .findAll('.settings-nav-item')
        .map((b) => b.text()),
    ).toContain('Power & dependencies')
  })

  it('lists the Append-only section for an admin only', () => {
    const labels = (props: Record<string, unknown>): string[] =>
      mount(props)
        .findAll('.settings-nav-item')
        .map((b) => b.text())
    expect(labels({})).toContain('Append-only')
    expect(labels({ isAdmin: false })).not.toContain('Append-only')
  })

  it('renders the append-only pane for its section', async () => {
    const wrapper = mount({ section: 'append-only' })
    await flushPromises()

    expect(wrapper.findComponent({ name: 'RepoAppendOnlyPane' }).exists()).toBe(true)
  })

  it('reports a change made in the append-only pane', async () => {
    const wrapper = mount({ section: 'append-only' })
    await flushPromises()

    wrapper.findComponent({ name: 'RepoAppendOnlyPane' }).vm.$emit('changed')

    expect(wrapper.emitted('changed')).toHaveLength(1)
  })

  it('hides the Power & dependencies section from a non-admin', () => {
    expect(
      mount({ isAdmin: false })
        .findAll('.settings-nav-item')
        .map((b) => b.text()),
    ).not.toContain('Power & dependencies')
  })

  // Power belongs to the repository host now: the repository shows what its
  // host does and links there, with no form of its own to save.
  it("shows its host's settings read-only for the power section", async () => {
    const wrapper = mount({ section: 'power' })
    await flushPromises()

    expect(wrapper.findComponent({ name: 'RepoHostSettingsSummary' }).exists()).toBe(true)
    expect(wrapper.findAll('button').map((b) => b.text().trim())).not.toContain('Edit')
    expect(wrapper.find(`a[href="/repo-hosts/${REPO.repo_host.id}?section=power"]`).exists()).toBe(
      true,
    )
  })
})
