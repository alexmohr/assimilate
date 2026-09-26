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
  it('lists the Power section for an admin', () => {
    expect(
      mount()
        .findAll('.settings-nav-item')
        .map((b) => b.text()),
    ).toContain('Power')
  })

  it('hides the Power section from a non-admin', () => {
    expect(
      mount({ isAdmin: false })
        .findAll('.settings-nav-item')
        .map((b) => b.text()),
    ).not.toContain('Power')
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
