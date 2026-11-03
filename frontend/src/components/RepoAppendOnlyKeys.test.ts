// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { appendOnlyStatusFixture } from '../test-utils/appendOnlyFixtures'
import { findButton } from '../test-utils/dom'
import RepoAppendOnlyKeys from './RepoAppendOnlyKeys.vue'
import type { AppendOnlyStatusResponse } from '../types/generated'

function mount(status: AppendOnlyStatusResponse = appendOnlyStatusFixture(), busy = false) {
  return renderWithPlugins(RepoAppendOnlyKeys, { props: { status, busy } })
}

describe('RepoAppendOnlyKeys', () => {
  it('shows when the keys were installed', () => {
    expect(mount().find('.badge--success').text()).toContain('Installed')
    expect(mount(appendOnlyStatusFixture({ installed_at: null })).text()).toContain('Not installed')
  })

  it('asks to write the keys again', async () => {
    const wrapper = mount()
    await findButton(wrapper, /^Reapply$/).trigger('click')
    expect(wrapper.emitted('reapply')).toHaveLength(1)
  })

  it('keeps the manual lines hidden until asked', async () => {
    const wrapper = mount()
    expect(wrapper.find('.detail-pre').exists()).toBe(false)

    await findButton(wrapper, /Show the lines/).trigger('click')
    const lines = wrapper.findAll('.detail-pre').map((p) => p.text())
    expect(lines).toEqual([
      'command="borg serve --append-only",restrict ssh-ed25519 AAAAbackup',
      'ssh-ed25519 AAAAmaintenance assimilate-maintenance',
    ])

    await findButton(wrapper, /Hide the lines/).trigger('click')
    expect(wrapper.find('.detail-pre').exists()).toBe(false)
  })

  it('copies a line', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    const wrapper = mount()
    await findButton(wrapper, /Show the lines/).trigger('click')
    await findButton(wrapper, /Copy line/).trigger('click')
    expect(writeText).toHaveBeenCalledWith(
      'command="borg serve --append-only",restrict ssh-ed25519 AAAAbackup',
    )
  })

  it('copies the maintenance line and says so', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    const wrapper = mount()
    await findButton(wrapper, /Show the lines/).trigger('click')
    await wrapper
      .findAll('button')
      .filter((b) => /Copy line/.test(b.text()))[1]
      .trigger('click')
    await flushPromises()

    expect(writeText).toHaveBeenCalledWith('ssh-ed25519 AAAAmaintenance assimilate-maintenance')
    expect(wrapper.text()).toContain('Copied')
  })
})
