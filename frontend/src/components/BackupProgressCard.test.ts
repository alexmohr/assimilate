// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import BackupProgressCard from './BackupProgressCard.vue'

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(BackupProgressCard, {
    props: {
      badge: 'dragon',
      repoId: null,
      archiveName: 'dragon-2026-08-26T08:45:24',
      elapsedSecs: 11,
      estimatedRemainingSecs: null,
      progress: { nfiles: 29098, originalSize: 370_800_000, currentPath: 'usr/share/locale' },
      cancelLoading: false,
      ...props,
    },
  })
}

describe('BackupProgressCard', () => {
  it('links the badge to the repository when a repo id is known', () => {
    const wrapper = mount({ repoId: 7 })
    expect(wrapper.find('.live-log-host-badge').attributes('href')).toBe('/repos/7')
  })

  it('falls back to a plain badge when no repo id is known', () => {
    const wrapper = mount({ repoId: null })
    const badge = wrapper.find('.live-log-host-badge')
    expect(badge.attributes('href')).toBeUndefined()
    expect(badge.text()).toBe('dragon')
  })

  it('shows no cancel button without a repo id to cancel against', () => {
    const wrapper = mount({ repoId: null })
    expect(wrapper.find('.live-log-header-actions button').exists()).toBe(false)
  })

  it('emits cancel when the cancel button is clicked', async () => {
    const wrapper = mount({ repoId: 7 })
    await wrapper.find('.live-log-header-actions button').trigger('click')
    expect(wrapper.emitted('cancel')).toHaveLength(1)
  })

  it('disables the cancel button and relabels it while cancelling', () => {
    const wrapper = mount({ repoId: 7, cancelLoading: true })
    const button = wrapper.find('.live-log-header-actions button')
    expect(button.attributes('disabled')).toBeDefined()
    expect(button.text()).toBe('Cancelling...')
  })

  // Whether or not clampPath is set, the full path always reaches the DOM -
  // clamping is a visual (CSS) bound, not truncated text - so it stays
  // copy-pasteable and readable by assistive tech either way.
  it('keeps the full current-file path in the DOM regardless of clampPath', () => {
    const longPath =
      'usr/share/locale/en_GB/LC_MESSAGES/plasma_applet_org.kde.plasma.digitalclock.mo'
    const wrapper = mount({
      progress: { nfiles: 1, originalSize: 0, currentPath: longPath },
    })
    expect(wrapper.find('.progress-path').text()).toBe(longPath)
  })

  it('does not clamp the current-file path by default', () => {
    const wrapper = mount()
    expect(wrapper.find('.progress-path').classes()).not.toContain('progress-path--clamp')
  })

  it('clamps the current-file path to two lines when clampPath is set', () => {
    const wrapper = mount({ clampPath: true })
    expect(wrapper.find('.progress-path').classes()).toContain('progress-path--clamp')
  })
})
