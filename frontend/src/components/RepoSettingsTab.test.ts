// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { repoFixture } from '../test-utils/repoFixtures'
import RepoSettingsTab from './RepoSettingsTab.vue'

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

  it('renders RepoPowerCard for the power section', () => {
    expect(mount({ section: 'power' }).findComponent({ name: 'RepoPowerCard' }).exists()).toBe(true)
  })

  it('forwards a save from the power form as changed', async () => {
    const wrapper = mount({ section: 'power' })
    wrapper.findComponent({ name: 'RepoPowerCard' }).vm.$emit('saved')
    await flushPromises()

    expect(wrapper.emitted('changed')).toHaveLength(1)
  })
})
