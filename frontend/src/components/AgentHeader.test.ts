// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import AgentHeader from './AgentHeader.vue'
import type { AgentRow } from '../types/agent'

const AGENT = {
  id: 1,
  hostname: 'web-01',
  display_name: 'Production Web',
  agent_version: '1.0.0',
  agent_git_sha: 'abc1234',
  agent_build_time: '2026-05-01',
  created_at: '2026-01-01T00:00:00Z',
  last_seen_at: '2026-06-01T00:00:00Z',
  is_connected: true,
  is_imported: false,
  supports_restart: true,
  restart_unavailable_reason: null,
} as unknown as AgentRow

function mount(agentOverrides: Record<string, unknown> = {}, props: Record<string, unknown> = {}) {
  return renderWithPlugins(AgentHeader, {
    props: {
      agent: { ...AGENT, ...agentOverrides },
      deployLabel: null,
      restartLoading: false,
      regenLoading: false,
      restartError: null,
      ...props,
    },
  })
}

async function openMenu(wrapper: ReturnType<typeof mount>) {
  await wrapper.find('.agent-menu-toggle').trigger('click')
  await flushPromises()
}

function menuLabels(wrapper: ReturnType<typeof mount>): string[] {
  return wrapper.findAll('.agent-menu-item').map((i) => i.text().trim())
}

/** Buttons on the header row itself, excluding the overflow toggle. */
function visibleActions(wrapper: ReturnType<typeof mount>): string[] {
  return wrapper
    .findAll('.agent-actions > button')
    .filter((b) => !b.classes().includes('agent-menu-toggle'))
    .map((b) => b.text().trim())
}

describe('AgentHeader', () => {
  it('shows the hostname, display name and status', () => {
    const wrapper = mount()
    expect(wrapper.find('.agent-hostname').text()).toBe('web-01')
    expect(wrapper.find('.agent-subtitle').text()).toBe('Production Web')
    expect(wrapper.find('.badge--success').text()).toContain('Online')
  })

  it('marks a disconnected agent offline', () => {
    expect(mount({ is_connected: false }).find('.badge--neutral').text()).toContain('Offline')
  })

  // The row used to hold up to eight identical ghost buttons. Now it holds
  // only the one thing that is actionable right now; everything else,
  // including navigation like Activity log, is one affordance away.
  it('keeps the visible row empty when no upgrade is available', () => {
    expect(visibleActions(mount())).toEqual([])
  })

  it('gives the accented slot to an available upgrade', () => {
    const wrapper = mount({}, { deployLabel: 'Upgrade' })
    expect(visibleActions(wrapper)).toEqual(['Upgrade agent'])
    expect(wrapper.find('.btn-primary').text()).toBe('Upgrade agent')
    expect(wrapper.find('.badge--info').text()).toBe('Upgrade available')
  })

  // Deploy is a first install rather than an upgrade, so it takes the slot
  // without claiming a newer build exists.
  it('does not claim an upgrade is available when the label is Deploy', () => {
    const wrapper = mount({}, { deployLabel: 'Deploy' })
    expect(wrapper.find('.btn-primary').text()).toBe('Deploy agent')
    expect(wrapper.find('.badge--info').exists()).toBe(false)
  })

  it('hides the rare actions until the overflow menu is opened', async () => {
    const wrapper = mount()
    expect(wrapper.findAll('.agent-menu-item')).toHaveLength(0)

    await openMenu(wrapper)

    expect(menuLabels(wrapper)).toEqual([
      'Activity log',
      'Edit identity',
      'Deploy SSH key',
      'Regenerate token',
      'Restart agent',
    ])
  })

  it('emits deploy from the visible row', async () => {
    const wrapper = mount({}, { deployLabel: 'Upgrade' })
    await wrapper
      .findAll('.agent-actions > button')
      .find((b) => b.text().trim() === 'Upgrade agent')!
      .trigger('click')

    expect(wrapper.emitted('deploy')).toHaveLength(1)
  })

  it('emits activityLog from the menu', async () => {
    const wrapper = mount()
    await openMenu(wrapper)
    await wrapper
      .findAll('.agent-menu-item')
      .find((i) => i.text().trim() === 'Activity log')!
      .trigger('click')

    expect(wrapper.emitted('activityLog')).toHaveLength(1)
  })

  // Escape and a click anywhere else both close the menu; a menu that only
  // closes by pressing its own toggle again is a trap.
  it('closes the menu on Escape', async () => {
    const wrapper = mount()
    await openMenu(wrapper)
    expect(wrapper.findAll('.agent-menu-item').length).toBeGreaterThan(0)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await flushPromises()

    expect(wrapper.findAll('.agent-menu-item')).toHaveLength(0)
  })

  it('closes the menu on a click outside it', async () => {
    const wrapper = mount()
    await openMenu(wrapper)

    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    await flushPromises()

    expect(wrapper.findAll('.agent-menu-item')).toHaveLength(0)
  })

  it('leaves the menu open when the click is inside it', async () => {
    const wrapper = mount()
    await openMenu(wrapper)

    wrapper
      .find('.agent-menu-item')
      .element.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    await flushPromises()

    expect(wrapper.findAll('.agent-menu-item').length).toBeGreaterThan(0)
  })

  it.each([
    ['Edit identity', 'editIdentity'],
    ['Deploy SSH key', 'deploySshKey'],
    ['Regenerate token', 'regenerateToken'],
    ['Restart agent', 'restart'],
  ])('emits %s from the menu', async (label, event) => {
    const wrapper = mount()
    await openMenu(wrapper)
    await wrapper
      .findAll('.agent-menu-item')
      .find((i) => i.text().trim() === label)!
      .trigger('click')

    expect(wrapper.emitted(event)).toHaveLength(1)
    // Acting closes the menu, so no item has to remember to.
    expect(wrapper.findAll('.agent-menu-item')).toHaveLength(0)
  })

  // Restart is offered only where it can work: it needs a supervisor that
  // supports it and an agent that is reachable.
  it.each([
    [{ supports_restart: false }, 'unsupported'],
    [{ is_connected: false }, 'offline'],
  ])('omits Restart agent when the agent is %s', async (overrides) => {
    const wrapper = mount(overrides)
    await openMenu(wrapper)
    expect(menuLabels(wrapper)).not.toContain('Restart agent')
  })

  it('explains why restart is unavailable when the agent said so', async () => {
    const wrapper = mount({
      supports_restart: false,
      restart_unavailable_reason: 'not managed by systemd',
    })
    await openMenu(wrapper)
    expect(wrapper.find('.agent-menu-note').text()).toBe('not managed by systemd')
  })

  it('reports the build the agent is running', () => {
    const meta = mount().find('.agent-meta').text()
    expect(meta).toContain('1.0.0')
    expect(meta).toContain('abc1234')
  })

  describe('imported hosts', () => {
    const IMPORTED = { is_imported: true, agent_version: null, agent_git_sha: null }

    // An imported host has exactly one job: to stop being one.
    it('gives the primary slots to adoption', () => {
      expect(visibleActions(mount(IMPORTED))).toEqual(['Adopt', 'Merge into...'])
    })

    it('labels it imported rather than online or offline', () => {
      const wrapper = mount(IMPORTED)
      expect(wrapper.find('.badge--neutral').text()).toBe('Imported')
      expect(wrapper.find('.badge--success').exists()).toBe(false)
    })

    // There is no agent binary on an imported host, so rendering em dashes for
    // version and revision would imply one is there but silent.
    it('omits the agent build meta entirely', () => {
      const meta = mount(IMPORTED).find('.agent-meta').text()
      expect(meta).not.toContain('agent')
      expect(meta).not.toContain('rev')
      expect(meta).toContain('added')
    })

    it.each([
      ['Adopt', 'adopt'],
      ['Merge into...', 'merge'],
    ])('emits %s', async (label, event) => {
      const wrapper = mount(IMPORTED)
      await wrapper
        .findAll('.agent-actions > button')
        .find((b) => b.text().trim() === label)!
        .trigger('click')

      expect(wrapper.emitted(event)).toHaveLength(1)
    })

    it('reaches the activity log through the menu instead', async () => {
      const wrapper = mount(IMPORTED)
      await openMenu(wrapper)
      await wrapper.find('.agent-menu-item').trigger('click')

      expect(wrapper.emitted('activityLog')).toHaveLength(1)
    })

    it('offers no agent-only actions in the menu', async () => {
      const wrapper = mount(IMPORTED)
      await openMenu(wrapper)
      expect(menuLabels(wrapper)).toEqual(['Activity log'])
    })
  })

  // A freshly created agent has connected but reported nothing about itself,
  // so every field in the meta strip is null at once.
  it('names the unknowns rather than leaving gaps', () => {
    const wrapper = mount({
      display_name: null,
      agent_version: null,
      agent_git_sha: null,
      agent_build_time: null,
      created_at: null,
      last_seen_at: null,
      is_connected: null,
    })

    expect(wrapper.find('.agent-subtitle').exists()).toBe(false)
    const meta = wrapper.find('.agent-meta').text()
    expect(meta).toContain('unknown')
    expect(meta).not.toContain('rev')
    expect(meta).not.toContain('seen')
    // A null connection state is not a connection.
    expect(wrapper.find('.badge--neutral').text()).toContain('Offline')
  })

  it('surfaces a failed restart', () => {
    expect(mount({}, { restartError: 'connection refused' }).find('.form-error').text()).toBe(
      'connection refused',
    )
  })
})
