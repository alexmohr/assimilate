// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import AgentDefaultsCards from './AgentDefaultsCards.vue'
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
  return renderWithPlugins(AgentDefaultsCards, {
    props: { agent: AGENT, canEdit: true, ...props },
  })
}

/** The nth `EditableInfoCard` on the panel, in template order. */
const CARDS = { paths: 0, excludes: 1, fileChange: 2, hooks: 3 }

function card(wrapper: ReturnType<typeof mount>, which: keyof typeof CARDS) {
  return wrapper.findAll('.info-card')[CARDS[which]]
}

describe('AgentDefaultsCards', () => {
  beforeEach(() => {
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.put).mockResolvedValue({ data: AGENT } as never)
  })

  it('renders one card per group of defaults', () => {
    const titles = mount()
      .findAll('.info-title')
      .map((t) => t.text())
    expect(titles).toEqual([
      'Default Backup Paths',
      'Default Exclude Patterns',
      'Default File Change Patterns',
      'Default Hook Commands',
    ])
  })

  it('lists the configured values and names the empty ones', () => {
    const wrapper = mount()
    expect(card(wrapper, 'paths').text()).toContain('/srv')
    expect(card(wrapper, 'paths').text()).toContain('/etc')
    expect(card(wrapper, 'excludes').text()).toContain('*.cache')
    expect(card(wrapper, 'hooks').text()).toContain('systemctl stop app')
    // Post-backup has none configured.
    expect(card(wrapper, 'hooks').text()).toContain('None configured.')
  })

  it('shows the empty message when a group has no values at all', () => {
    const wrapper = mount({
      agent: { ...AGENT, default_backup_paths: [], default_exclude_patterns: [] },
    })
    expect(card(wrapper, 'paths').text()).toContain('No default paths configured.')
    expect(card(wrapper, 'excludes').text()).toContain('No default excludes configured.')
  })

  it('renders the file change patterns with their action', () => {
    expect(card(mount(), 'fileChange').text()).toContain('/data/wal/**')
    expect(card(mount(), 'fileChange').text()).toContain('ignore')
  })

  it('hides every Edit button for an imported host', () => {
    const wrapper = mount({ canEdit: false })
    expect(wrapper.findAll('button')).toHaveLength(0)
  })

  it('seeds the textarea from the current value when editing starts', async () => {
    const wrapper = mount()
    await card(wrapper, 'paths').find('button').trigger('click')
    expect(card(wrapper, 'paths').find('textarea').element.value).toBe('/srv\n/etc')
  })

  it('sends the whole defaults payload with only the edited field replaced', async () => {
    const wrapper = mount()
    await card(wrapper, 'paths').find('button').trigger('click')
    await card(wrapper, 'paths').find('textarea').setValue('/var\n/opt')
    await card(wrapper, 'paths').findAll('button')[1].trigger('click')
    await flushPromises()

    expect(apiClient.put).toHaveBeenCalledWith('/agents/web-01', {
      display_name: 'Web 01',
      default_backup_paths: ['/var', '/opt'],
      default_exclude_patterns: ['*.cache'],
      default_pre_backup_commands: ['systemctl stop app'],
      default_post_backup_commands: [],
      default_file_change_patterns_raw: '/data/wal/** ignore',
    })
  })

  it('sends both hook command lists together', async () => {
    const wrapper = mount()
    await card(wrapper, 'hooks').find('button').trigger('click')
    const areas = card(wrapper, 'hooks').findAll('textarea')
    await areas[0].setValue('pre-one\npre-two')
    await areas[1].setValue('post-one')
    await card(wrapper, 'hooks').findAll('button')[1].trigger('click')
    await flushPromises()

    expect(vi.mocked(apiClient.put).mock.calls[0][1]).toMatchObject({
      default_pre_backup_commands: ['pre-one', 'pre-two'],
      default_post_backup_commands: ['post-one'],
    })
  })

  it('emits the saved agent so the view can merge it back', async () => {
    const wrapper = mount()
    await card(wrapper, 'paths').find('button').trigger('click')
    await card(wrapper, 'paths').findAll('button')[1].trigger('click')
    await flushPromises()

    expect(wrapper.emitted('saved')).toEqual([[AGENT]])
  })

  it('stays in edit mode and shows the error when the save fails', async () => {
    vi.mocked(apiClient.put).mockRejectedValue(new Error('agent offline'))
    const wrapper = mount()
    await card(wrapper, 'paths').find('button').trigger('click')
    await card(wrapper, 'paths').findAll('button')[1].trigger('click')
    await flushPromises()

    expect(card(wrapper, 'paths').find('.form-error').text()).toContain('agent offline')
    expect(card(wrapper, 'paths').find('textarea').exists()).toBe(true)
  })

  it('leaves edit mode after a successful save', async () => {
    const wrapper = mount()
    await card(wrapper, 'paths').find('button').trigger('click')
    await card(wrapper, 'paths').findAll('button')[1].trigger('click')
    await flushPromises()

    expect(card(wrapper, 'paths').find('textarea').exists()).toBe(false)
  })

  it('keeps each card edit state independent', async () => {
    const wrapper = mount()
    await card(wrapper, 'paths').find('button').trigger('click')

    expect(card(wrapper, 'paths').find('textarea').exists()).toBe(true)
    expect(card(wrapper, 'excludes').find('textarea').exists()).toBe(false)
  })
})
