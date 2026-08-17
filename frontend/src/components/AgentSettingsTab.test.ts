// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import AgentSettingsTab from './AgentSettingsTab.vue'
import type { AgentRow } from '../types/agent'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), put: vi.fn(), post: vi.fn(), delete: vi.fn() },
}))

const AGENT = {
  id: 1,
  hostname: 'web-01',
  display_name: 'Production Web',
  agent_version: '1.0.0',
  agent_git_sha: 'abc1234',
  agent_build_time: '2026-05-01',
  created_at: '2026-01-01T00:00:00Z',
  last_seen_at: '2026-06-01T00:00:00Z',
  is_imported: false,
  default_backup_paths: [],
  default_exclude_patterns: [],
  default_pre_backup_commands: [],
  default_post_backup_commands: [],
  default_file_change_patterns_raw: '',
} as unknown as AgentRow

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentSettingsTab, {
    props: {
      agent: AGENT,
      section: 'identity',
      isAdmin: true,
      regenLoading: false,
      ...props,
    },
  })
}

function clickButton(wrapper: ReturnType<typeof mount>, label: string) {
  const button = wrapper.findAll('button').find((b) => b.text().trim() === label)
  if (!button) throw new Error(`no "${label}" button`)
  return button.trigger('click')
}

describe('AgentSettingsTab', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get)
      .mockReset()
      .mockResolvedValue({ data: [] } as never)
  })

  it('lists every settings section for an admin', () => {
    expect(
      mount()
        .findAll('.settings-nav-item')
        .map((b) => b.text()),
    ).toEqual(['Identity', 'Backup defaults', 'Hostname aliases', 'Tags', 'Danger zone'])
  })

  // Tags and the danger zone are admin-only, so they are absent rather than
  // present-and-disabled.
  it('hides the admin-only sections from everyone else', () => {
    expect(
      mount({ isAdmin: false })
        .findAll('.settings-nav-item')
        .map((b) => b.text()),
    ).toEqual(['Identity', 'Backup defaults', 'Hostname aliases'])
  })

  it('marks the current section', () => {
    const wrapper = mount({ section: 'aliases' })
    const current = wrapper
      .findAll('.settings-nav-item')
      .find((b) => b.attributes('aria-current') === 'true')
    expect(current!.text()).toBe('Hostname aliases')
  })

  it('asks the view to record the chosen section', async () => {
    const wrapper = mount()
    await wrapper
      .findAll('.settings-nav-item')
      .find((b) => b.text() === 'Danger zone')!
      .trigger('click')

    expect(wrapper.emitted('update:section')).toEqual([['danger']])
  })

  describe('identity', () => {
    it('shows the agent build details', () => {
      const text = mount().text()
      expect(text).toContain('1.0.0')
      expect(text).toContain('abc1234')
    })

    it('opens the identity dialog from Edit', async () => {
      const wrapper = mount()
      await clickButton(wrapper, 'Edit')
      expect(wrapper.emitted('editIdentity')).toHaveLength(1)
    })

    it('asks the view to regenerate the token', async () => {
      const wrapper = mount()
      await clickButton(wrapper, 'Regenerate token')
      expect(wrapper.emitted('regenerateToken')).toHaveLength(1)
    })

    it('disables the token button while a regeneration is in flight', () => {
      const wrapper = mount({ regenLoading: true })
      const button = wrapper.findAll('button').find((b) => b.text().includes('Regenerating'))
      expect(button!.attributes('disabled')).toBeDefined()
    })

    it('names the unknowns on an agent that has reported nothing', () => {
      const text = mount({
        agent: {
          ...AGENT,
          display_name: null,
          agent_version: null,
          agent_git_sha: null,
          agent_build_time: null,
          created_at: null,
          last_seen_at: null,
        },
      }).text()

      expect(text).toContain('Not set')
      expect(text).toContain('Unknown')
      expect(text).toContain('Never')
    })

    // An imported host has no agent to hold a token or report a build.
    it('omits the connection card and build rows for an imported host', () => {
      const wrapper = mount({ agent: { ...AGENT, is_imported: true } })
      expect(wrapper.text()).not.toContain('Connection')
      expect(wrapper.text()).not.toContain('Agent version')
      expect(wrapper.findAll('button').some((b) => b.text().trim() === 'Edit')).toBe(false)
    })
  })

  it('forwards a saved agent from the defaults form', async () => {
    const wrapper = mount({ section: 'defaults' })
    wrapper.findComponent({ name: 'AgentDefaultsCard' }).vm.$emit('saved', AGENT)
    await flushPromises()

    expect(wrapper.emitted('saved')).toEqual([[AGENT]])
  })

  // The view renames the agent, then offers to keep the old hostname as a
  // pattern; accepting that has to refresh the list this tab owns.
  it('reloads the alias list on request', async () => {
    const wrapper = mount({ section: 'aliases' })
    await flushPromises()
    vi.mocked(apiClient.get).mockClear()

    await (wrapper.vm as unknown as { reloadAliases: (h: string) => Promise<void> }).reloadAliases(
      'renamed-host',
    )
    await flushPromises()

    expect(apiClient.get).toHaveBeenCalledWith(
      expect.stringContaining('renamed-host/hostname-patterns'),
    )
  })
})
