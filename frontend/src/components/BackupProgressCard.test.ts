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

  // The scoped `.progress-path` CSS clamps this to two lines and ellipsizes
  // any overflow visually; the full path still reaches the DOM so it stays
  // copy-pasteable and readable by assistive tech.
  it('keeps the full current-file path in the DOM for the CSS clamp to bound visually', () => {
    const longPath =
      'usr/share/locale/en_GB/LC_MESSAGES/plasma_applet_org.kde.plasma.digitalclock.mo'
    const wrapper = mount({
      progress: { nfiles: 1, originalSize: 0, currentPath: longPath },
    })
    expect(wrapper.find('.progress-path').text()).toBe(longPath)
  })
})
