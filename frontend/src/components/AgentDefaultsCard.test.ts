// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import {
  clickSectionButton,
  expectSaveErrorKeepsEditing,
  expectSavedEmitted,
  renderWithPlugins,
  startEditingSection,
} from '../test-utils'
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

  // Older/imported agent rows may have never had these fields set, unlike
  // the fixture's always-present arrays - editing must still start cleanly
  // rather than seeding the command lists with undefined.
  it('starts editing with empty command lists when the fields are missing entirely', async () => {
    const agent = { ...AGENT }
    delete (agent as { default_pre_backup_commands?: string[] }).default_pre_backup_commands
    delete (agent as { default_post_backup_commands?: string[] }).default_post_backup_commands
    const wrapper = mount({ agent })
    await startEditingSection(wrapper)

    const editors = wrapper.findAllComponents({ name: 'CommandListEditor' })
    expect(editors[0].props('modelValue')).toEqual([])
    expect(editors[1].props('modelValue')).toEqual([])
  })

  it('seeds every field from the current value when editing starts', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)

    expect(wrapper.find<HTMLTextAreaElement>('#defaults-paths').element.value).toBe('/srv\n/etc')
    expect(wrapper.find<HTMLTextAreaElement>('#defaults-excludes').element.value).toBe('*.cache')
    const editors = wrapper.findAllComponents({ name: 'CommandListEditor' })
    expect(editors[0].props('modelValue')).toEqual(['systemctl stop app'])
    expect(editors[1].props('modelValue')).toEqual([])
    // Each command list still carries an accessible name, since swapping the
    // old single `<textarea id>` for a variable-length list of fields means
    // a single `<label for>` can no longer target them directly.
    expect(editors[0].props('ariaLabel')).toBe('Pre-backup commands')
    expect(editors[1].props('ariaLabel')).toBe('Post-backup commands')
    expect(wrapper.findComponent({ name: 'FileChangePatternsEditor' }).props('modelValue')).toBe(
      '/data/wal/** ignore',
    )
  })

  // The PUT is a whole-object replace. One form means one request carrying
  // every field, rather than four requests each echoing the other three.
  it('sends every defaults field in a single request', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('#defaults-paths').setValue('/var\n/opt')
    await wrapper
      .findAllComponents({ name: 'CommandListEditor' })[1]
      .vm.$emit('update:modelValue', ['systemctl start app'])
    await clickSectionButton(wrapper, 'Save')

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
    await startEditingSection(wrapper)
    const editors = wrapper.findAllComponents({ name: 'CommandListEditor' })
    await editors[0].vm.$emit('update:modelValue', ['pre-one', 'pre-two'])
    await editors[1].vm.$emit('update:modelValue', ['post-one'])
    await clickSectionButton(wrapper, 'Save')

    expect(vi.mocked(apiClient.put).mock.calls[0][1]).toMatchObject({
      default_pre_backup_commands: ['pre-one', 'pre-two'],
      default_post_backup_commands: ['post-one'],
    })
  })

  it('emits the saved agent so the view can merge it back', async () => {
    const wrapper = mount()
    await expectSavedEmitted(wrapper, [AGENT])
  })

  it('stays in edit mode and shows the error when the save fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('agent offline'))
    const wrapper = mount()
    await expectSaveErrorKeepsEditing(wrapper, 'agent offline', '#defaults-paths')
  })

  it('leaves edit mode after a successful save', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await clickSectionButton(wrapper, 'Save')

    expect(wrapper.find('#defaults-paths').exists()).toBe(false)
  })

  it('leaves the card without saving on Cancel', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    const saveVisible = () => wrapper.findAll('button').some((b) => b.text().trim() === 'Save')
    expect(saveVisible()).toBe(true)

    await clickSectionButton(wrapper, 'Cancel')

    expect(saveVisible()).toBe(false)
    expect(apiClient.put).not.toHaveBeenCalled()
  })

  it('saves edited exclude patterns as a list, dropping blank lines', async () => {
    const wrapper = mount()
    await startEditingSection(wrapper)
    await wrapper.find('#defaults-excludes').setValue('*.cache\n\n/var/tmp\n')

    await clickSectionButton(wrapper, 'Save')

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
    await startEditingSection(wrapper)
    await wrapper
      .findComponent({ name: 'FileChangePatternsEditor' })
      .vm.$emit('update:modelValue', '/data/wal/** ignore\n/srv/** watch')
    await flushPromises()

    await clickSectionButton(wrapper, 'Save')

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
    await startEditingSection(wrapper)
    await wrapper.find('#defaults-excludes').setValue('discarded')
    await clickSectionButton(wrapper, 'Cancel')

    await startEditingSection(wrapper)
    expect(wrapper.find<HTMLTextAreaElement>('#defaults-excludes').element.value).toBe('*.cache')
  })
})
