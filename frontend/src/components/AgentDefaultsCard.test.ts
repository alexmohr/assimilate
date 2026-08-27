// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import AgentDefaultsCard from './AgentDefaultsCard.vue'
import type { AgentRow } from '../types/agent'

vi.mock('../api/client', () => ({
  apiClient: { put: vi.fn() },
}))

const AGENT = {
  hostname: 'web-01',
  display_name: 'Web 01',
  default_backup_paths: ['/srv', '/etc'],
  default_exclude_patterns: ['*.cache'],
  default_pre_backup_commands: ['systemctl stop app'],
  default_post_backup_commands: [],
  default_file_change_patterns_raw: '/data/wal/** ignore',
  is_imported: false,
} as unknown as AgentRow

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentDefaultsCard, {
    props: { agent: AGENT, canEdit: true, ...props },
  })
}

async function startEditing(wrapper: ReturnType<typeof mount>) {
  await wrapper.find('button').trigger('click')
}

async function clickButton(wrapper: ReturnType<typeof mount>, label: string) {
  const button = wrapper.findAll('button').find((b) => b.text().trim() === label)
  if (!button) throw new Error(`no "${label}" button on the card`)
  await button.trigger('click')
  await flushPromises()
}

describe('AgentDefaultsCard', () => {
  beforeEach(() => {
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.put).mockResolvedValue({ data: AGENT } as never)
  })

  // The four separate cards became one pane with five labelled sections, so
  // saving can no longer write a stale copy of a sibling field. The settings
  // rail names the pane, so the pane itself carries a lede rather than a
  // heading repeating that name.
  it('renders a single pane holding every group of defaults', () => {
    const wrapper = mount()
    expect(wrapper.findAll('.pane-head')).toHaveLength(1)
    expect(wrapper.find('.pane-lede').text()).toContain('What a schedule uses for this host')
    expect(wrapper.findAll('.group-label').map((l) => l.text())).toEqual([
      'Backup paths',
      'Exclude patterns',
      'File change patterns',
      'Pre-backup commands',
      'Post-backup commands',
    ])
  })

  it('lists the configured values and names the empty ones', () => {
    const text = mount().text()
    expect(text).toContain('/srv')
    expect(text).toContain('/etc')
    expect(text).toContain('*.cache')
    expect(text).toContain('systemctl stop app')
    // Post-backup has none configured.
    expect(text).toContain('None configured.')
  })

  it('lists both hook command groups when both are configured', () => {
    const wrapper = mount({
      agent: { ...AGENT, default_post_backup_commands: ['systemctl start app', 'notify-send ok'] },
    })

    const commands = wrapper.findAll('code.path-item').map((c) => c.text())
    expect(commands).toContain('systemctl stop app')
    expect(commands).toContain('systemctl start app')
    expect(commands).toContain('notify-send ok')
    expect(wrapper.text()).not.toContain('None configured.')
  })

  it('shows the empty message when a group has no values at all', () => {
    const wrapper = mount({
      agent: { ...AGENT, default_backup_paths: [], default_exclude_patterns: [] },
    })
    expect(wrapper.text()).toContain('No default paths configured.')
    expect(wrapper.text()).toContain('No default excludes configured.')
  })

  it('renders the file change patterns with their action', () => {
    const text = mount().text()
    expect(text).toContain('/data/wal/**')
    expect(text).toContain('ignore')
  })

  it('hides the Edit button for an imported host', () => {
    expect(mount({ canEdit: false }).findAll('button')).toHaveLength(0)
  })

  it('seeds every field from the current value when editing starts', async () => {
    const wrapper = mount()
    await startEditing(wrapper)

    expect(wrapper.find<HTMLTextAreaElement>('#defaults-paths').element.value).toBe('/srv\n/etc')
    expect(wrapper.find<HTMLTextAreaElement>('#defaults-excludes').element.value).toBe('*.cache')
    expect(wrapper.find<HTMLTextAreaElement>('#defaults-pre').element.value).toBe(
      'systemctl stop app',
    )
    expect(wrapper.find<HTMLTextAreaElement>('#defaults-post').element.value).toBe('')
    expect(wrapper.findComponent({ name: 'FileChangePatternsEditor' }).props('modelValue')).toBe(
      '/data/wal/** ignore',
    )
  })

  // The PUT is a whole-object replace. One form means one request carrying
  // every field, rather than four requests each echoing the other three.
  it('sends every defaults field in a single request', async () => {
    const wrapper = mount()
    await startEditing(wrapper)
    await wrapper.find('#defaults-paths').setValue('/var\n/opt')
    await wrapper.find('#defaults-post').setValue('systemctl start app')
    await clickButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledTimes(1)
    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/web-01',
      {
        display_name: 'Web 01',
        domain: undefined,
        default_backup_paths: ['/var', '/opt'],
        default_exclude_patterns: ['*.cache'],
        default_pre_backup_commands: ['systemctl stop app'],
        default_post_backup_commands: ['systemctl start app'],
        default_file_change_patterns_raw: '/data/wal/** ignore',
      },
      { params: {} },
    )
  })

  it('sends both hook command lists together', async () => {
    const wrapper = mount()
    await startEditing(wrapper)
    await wrapper.find('#defaults-pre').setValue('pre-one\npre-two')
    await wrapper.find('#defaults-post').setValue('post-one')
    await clickButton(wrapper, 'Save')

    expect(vi.mocked(apiClient.put).mock.calls[0][1]).toMatchObject({
      default_pre_backup_commands: ['pre-one', 'pre-two'],
      default_post_backup_commands: ['post-one'],
    })
  })

  it('emits the saved agent so the view can merge it back', async () => {
    const wrapper = mount()
    await startEditing(wrapper)
    await clickButton(wrapper, 'Save')

    expect(wrapper.emitted('saved')).toEqual([[AGENT]])
  })

  it('stays in edit mode and shows the error when the save fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('agent offline'))
    const wrapper = mount()
    await startEditing(wrapper)
    await clickButton(wrapper, 'Save')

    expect(wrapper.find('.form-error').text()).toContain('agent offline')
    expect(wrapper.find('#defaults-paths').exists()).toBe(true)
  })

  it('leaves edit mode after a successful save', async () => {
    const wrapper = mount()
    await startEditing(wrapper)
    await clickButton(wrapper, 'Save')

    expect(wrapper.find('#defaults-paths').exists()).toBe(false)
  })

  it('leaves the card without saving on Cancel', async () => {
    const wrapper = mount()
    await startEditing(wrapper)
    const saveVisible = () => wrapper.findAll('button').some((b) => b.text().trim() === 'Save')
    expect(saveVisible()).toBe(true)

    await clickButton(wrapper, 'Cancel')

    expect(saveVisible()).toBe(false)
    expect(apiClient.put).not.toHaveBeenCalled()
  })

  it('saves edited exclude patterns as a list, dropping blank lines', async () => {
    const wrapper = mount()
    await startEditing(wrapper)
    await wrapper.find('#defaults-excludes').setValue('*.cache\n\n/var/tmp\n')

    await clickButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/web-01',
      expect.objectContaining({ default_exclude_patterns: ['*.cache', '/var/tmp'] }),
      { params: {} },
    )
  })

  // File change patterns are stored as raw text, not split into a list, so
  // they must not get the line-splitting treatment the other groups do.
  it('saves file change patterns as raw text', async () => {
    const wrapper = mount()
    await startEditing(wrapper)
    await wrapper
      .findComponent({ name: 'FileChangePatternsEditor' })
      .vm.$emit('update:modelValue', '/data/wal/** ignore\n/srv/** watch')
    await flushPromises()

    await clickButton(wrapper, 'Save')

    expect(apiClient.put).toHaveBeenCalledWith(
      '/agents/web-01',
      expect.objectContaining({
        default_file_change_patterns_raw: '/data/wal/** ignore\n/srv/** watch',
      }),
      { params: {} },
    )
  })

  it('reopens a cancelled card with the stored value, not the discarded edit', async () => {
    const wrapper = mount()
    await startEditing(wrapper)
    await wrapper.find('#defaults-excludes').setValue('discarded')
    await clickButton(wrapper, 'Cancel')

    await startEditing(wrapper)
    expect(wrapper.find<HTMLTextAreaElement>('#defaults-excludes').element.value).toBe('*.cache')
  })
})
