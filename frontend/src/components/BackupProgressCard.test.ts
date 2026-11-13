// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { flushPromises } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import { appendHookOutput, MAX_HOOK_OUTPUT_LINES } from '../utils/hookOutput'
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

  it('says a run held for an offline host is queued, not in progress', () => {
    const wrapper = mount({ waitingFor: 'dragon', progress: null })
    expect(wrapper.find('.live-log-title').text()).toBe('Backup queued')
    expect(wrapper.find('.pulse-dot--waiting').exists()).toBe(true)
    expect(wrapper.text()).toContain(
      'dragon is offline. The backup starts as soon as it reconnects.',
    )
    expect(wrapper.text()).not.toContain('Waiting for progress')
  })

  /** A backup that got going and then lost its host keeps its last known progress. */
  it('keeps showing progress when a run that had started loses its host', () => {
    const wrapper = mount({ waitingFor: 'dragon' })
    expect(wrapper.find('.live-log-title').text()).toBe('Backup in progress')
    expect(wrapper.find('.pulse-dot--waiting').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('is offline')
  })

  it('shows what a script printed instead of waiting for progress', () => {
    const wrapper = mount({
      progress: null,
      hookOutput: [
        { type: 'hook_output', phase: 'pre_backup', stream: 'stdout', line: 'dumping database' },
        { type: 'hook_output', phase: 'pre_backup', stream: 'stderr', line: 'disk almost full' },
      ],
    })
    expect(wrapper.text()).not.toContain('Waiting for progress')
    expect(wrapper.find('.hook-output-log').text()).toContain('dumping database')
    expect(wrapper.find('.hook-output-line--stderr').text()).toContain('disk almost full')
  })

  it('shows script output beside the progress once borg is running', () => {
    const wrapper = mount({
      hookOutput: [{ type: 'hook_output', phase: 'pre_backup', stream: 'stdout', line: 'ready' }],
    })
    expect(wrapper.text()).toContain('Files')
    expect(wrapper.find('.hook-output-log').text()).toContain('ready')
  })

  it('scrolls the script output to the newest line as lines arrive', async () => {
    const first = {
      type: 'hook_output',
      phase: 'pre_backup',
      stream: 'stdout',
      line: 'first',
    } as const
    const wrapper = mount({ progress: null, hookOutput: [first] })
    const log = wrapper.find('.hook-output-log').element as HTMLElement
    Object.defineProperty(log, 'scrollHeight', { value: 480, configurable: true })
    expect(log.scrollTop).toBe(0)

    await wrapper.setProps({ hookOutput: [first, { ...first, line: 'second' }] })
    await flushPromises()

    expect(log.scrollTop).toBe(480)
  })

  it('keeps following the newest line once the output is at its cap', async () => {
    const lines = Array.from({ length: MAX_HOOK_OUTPUT_LINES }, (_, i) => ({
      type: 'hook_output' as const,
      phase: 'pre_backup' as const,
      stream: 'stdout' as const,
      line: `line ${i}`,
    }))
    const wrapper = mount({ progress: null, hookOutput: lines })
    const log = wrapper.find('.hook-output-log').element as HTMLElement
    Object.defineProperty(log, 'scrollHeight', { value: 900, configurable: true })
    expect(log.scrollTop).toBe(0)

    // One line in, the oldest out: the count is unchanged.
    const next = appendHookOutput(lines, { ...lines[0]!, line: 'newest' })
    expect(next).toHaveLength(MAX_HOOK_OUTPUT_LINES)
    await wrapper.setProps({ hookOutput: next })
    await flushPromises()

    expect(log.scrollTop).toBe(900)
  })

  it('has no script output section when no script printed anything', () => {
    expect(mount().find('.hook-output').exists()).toBe(false)
  })

  it('hides script output for a run that is only queued', () => {
    const wrapper = mount({
      waitingFor: 'dragon',
      progress: null,
      hookOutput: [{ type: 'hook_output', phase: 'pre_backup', stream: 'stdout', line: 'x' }],
    })
    expect(wrapper.find('.hook-output').exists()).toBe(false)
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

  it('shows a waiting message when no progress has arrived yet', () => {
    const wrapper = mount({ progress: null })
    expect(wrapper.text()).toContain('Waiting for progress')
  })

  it('falls back to 0 files instead of crashing when nfiles is missing', () => {
    // Regression test: a malformed progress payload (e.g. borg's final
    // archive_progress line, which omits nfiles) must not crash rendering.
    const progress = { originalSize: 1024, currentPath: '' } as unknown as {
      nfiles: number
      originalSize: number
      currentPath: string
    }
    const wrapper = mount({ progress })
    expect(wrapper.text()).toContain('0')
  })
})
