// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount, RouterLinkStub } from '@vue/test-utils'
import ArchiveSelectorRow from './ArchiveSelectorRow.vue'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'

function archive(overrides: Partial<ArchiveEntry> = {}): ArchiveEntry {
  return {
    name: 'web-01-2026-03-01',
    start: '2026-03-01T02:00:00Z',
    hostname: 'web-01',
    comment: '',
    original_size: 2048,
    deduplicated_size: 1024,
    matched: true,
    agent_hostname: 'web-01',
    ...overrides,
  }
}

function row(props: Record<string, unknown> = {}) {
  return mount(ArchiveSelectorRow, {
    global: { stubs: { RouterLink: RouterLinkStub } },
    props: {
      archive: archive(),
      selected: false,
      flat: false,
      canDelete: true,
      deleting: false,
      ...props,
    },
  })
}

describe('ArchiveSelectorRow', () => {
  it('leads with the name and size, then the host, date and dedup size', () => {
    const wrapper = row()
    expect(wrapper.find('.archive-name').text()).toBe('web-01-2026-03-01')
    expect(wrapper.find('.archive-size').text()).toBe('2.0 KB')
    expect(wrapper.find('.archive-host').text()).toBe('web-01')
    expect(wrapper.find('.archive-dedup').text()).toContain('1.0 KB')
  })

  it('prefers the agent hostname over the one borg recorded', () => {
    const wrapper = row({
      archive: archive({ hostname: 'raw-name', agent_hostname: 'web-01.internal' }),
    })
    expect(wrapper.find('.archive-host').text()).toBe('web-01.internal')
  })

  it('marks an archive no agent claims, in the flat list as well as grouped', () => {
    const wrapper = row({
      archive: archive({ matched: false, agent_hostname: null, hostname: 'legacy-nas' }),
      flat: true,
    })
    expect(wrapper.classes()).toContain('archive-row--unmatched')
    expect(wrapper.find('.archive-host').text()).toBe('legacy-nas')
  })

  it('links the host to its agent, from the row itself', () => {
    // The host used to be a link everywhere it appeared; when the row became a
    // single button it silently became plain text, and getting to the agent
    // meant selecting the archive first.
    const link = row().findComponent(RouterLinkStub)
    expect(link.props('to')).toBe('/agents/web-01')
    expect(link.text()).toBe('web-01')
  })

  it('links a host no agent claims too, so the name is still followable', () => {
    const wrapper = row({
      archive: archive({ matched: false, agent_hostname: null, hostname: 'legacy nas' }),
    })
    expect(wrapper.findComponent(RouterLinkStub).props('to')).toBe('/agents/legacy%20nas')
  })

  it('selects through a real button, so the row is reachable by keyboard', async () => {
    const wrapper = row()
    const select = wrapper.find('button.archive-row-select')
    expect(select.exists()).toBe(true)
    await select.trigger('click')
    expect(wrapper.emitted('select')).toHaveLength(1)
  })

  it('selects when the archive name itself is clicked, not only the button', async () => {
    // The button is stretched over the row for focus, and if it took pointer
    // events it would sit between every click and the text under it - which is
    // how a click on the name reached the button instead of the row and the
    // e2e suite reported "archive-row-select intercepts pointer events".
    const wrapper = row()
    await wrapper.find('.archive-name').trigger('click')
    expect(wrapper.emitted('select')).toHaveLength(1)

    await wrapper.find('.archive-date').trigger('click')
    expect(wrapper.emitted('select')).toHaveLength(2)
  })

  it('follows the host link without also selecting the row', async () => {
    const wrapper = row()
    await wrapper.find('.archive-host').trigger('click')
    expect(wrapper.emitted('select')).toBeUndefined()
  })

  it('deletes without also selecting the row', async () => {
    const wrapper = row()
    await wrapper.find('.archive-row-delete').trigger('click')
    expect(wrapper.emitted('delete')).toHaveLength(1)
    expect(wrapper.emitted('select')).toBeUndefined()
  })

  it('always renders the delete control for a user who may delete', async () => {
    // It used to be `opacity: 0` until the pointer landed on the row, which
    // made it invisible and unreachable without a mouse.
    const wrapper = row()
    const del = wrapper.find('.archive-row-delete')
    expect(del.exists()).toBe(true)
    expect(del.attributes('title')).toBe('Delete archive')
    await del.trigger('click')
    expect(wrapper.emitted('delete')).toHaveLength(1)
  })

  it('renders no delete control at all when the user may not delete', () => {
    expect(row({ canDelete: false }).find('.archive-row-delete').exists()).toBe(false)
  })

  it('names the in-flight delete and blocks a second one', async () => {
    const wrapper = row({ deleting: true })
    const del = wrapper.find('.archive-row-delete')
    expect(wrapper.find('.archive-row-pending').text()).toBe('Deleting')
    expect(del.attributes('title')).toBe('Deletion in progress')
    expect(del.attributes('disabled')).toBeDefined()
  })

  it('carries the flat modifier only in the flat list', () => {
    expect(row({ flat: true }).classes()).toContain('archive-row-detailed')
    expect(row({ flat: false }).classes()).not.toContain('archive-row-detailed')
  })

  it('marks the selected row', () => {
    expect(row({ selected: true }).classes()).toContain('selected')
    expect(row().classes()).not.toContain('selected')
  })
})
