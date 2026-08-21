// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { mount, RouterLinkStub } from '@vue/test-utils'
import { nextTick } from 'vue'
import ArchiveSelector from './ArchiveSelector.vue'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'

function archive(overrides: Partial<ArchiveEntry> = {}): ArchiveEntry {
  return {
    name: 'web-01-2026-03-01',
    start: '2026-03-01T02:00:00Z',
    hostname: 'web-01',
    comment: '',
    original_size: 1000,
    deduplicated_size: 100,
    matched: true,
    agent_hostname: 'web-01',
    ...overrides,
  }
}

const WEB = archive()
const DB = archive({
  name: 'db-01-2026-02-01',
  start: '2026-02-01T02:00:00Z',
  hostname: 'db-01',
  agent_hostname: 'db-01',
  original_size: 3000,
  deduplicated_size: 300,
})

function selector(props: Record<string, unknown> = {}) {
  return mount(ArchiveSelector, {
    global: { stubs: { RouterLink: RouterLinkStub } },
    props: {
      archives: [WEB, DB],
      selected: null,
      ...props,
    },
  })
}

async function pickGroupMode(wrapper: ReturnType<typeof selector>, label: string): Promise<void> {
  const option = wrapper
    .findAll('.archive-group-toggle .segmented-option')
    .find((b) => b.text() === label)
  expect(option).toBeDefined()
  await option!.trigger('click')
  await nextTick()
}

describe('ArchiveSelector', () => {
  it('groups by host and totals each group', () => {
    const wrapper = selector()
    const groups = wrapper.findAll('.archive-group')
    expect(groups).toHaveLength(2)
    // Groups sort by hostname, so db-01 leads.
    expect(groups[0].find('.group-hostname').text()).toBe('db-01')
    expect(groups[0].find('.group-count').text()).toBe('1')
    expect(groups[0].find('.group-size').text()).toBe('2.9 KB')
  })

  it('links a group host to its agent without swallowing the collapse toggle', async () => {
    const wrapper = selector()
    const group = wrapper.findAll('.archive-group')[0]

    const link = group.findComponent(RouterLinkStub)
    expect(link.props('to')).toBe('/agents/db-01')
    expect(link.text()).toBe('db-01')

    // The link lives inside the header, so the header can no longer be the
    // button; the header takes the click and the toggle beside the link
    // carries the state for a screen reader.
    expect(group.find('.group-header').classes()).not.toContain('collapsed')
    await group.find('.group-toggle').trigger('click')
    expect(group.find('.group-header').classes()).toContain('collapsed')

    // Clicking the header anywhere else toggles it back...
    await group.find('.group-count').trigger('click')
    expect(group.find('.group-header').classes()).not.toContain('collapsed')

    // ...but following the host link must not collapse the group under it.
    await link.trigger('click')
    expect(group.find('.group-header').classes()).not.toContain('collapsed')
  })

  it('switches to a flat list through the segmented control', async () => {
    const wrapper = selector()
    expect(wrapper.find('.archive-groups').exists()).toBe(true)

    await pickGroupMode(wrapper, 'Flat')

    expect(wrapper.find('.archive-flat-list').exists()).toBe(true)
    expect(wrapper.find('.archive-groups').exists()).toBe(false)
    expect(wrapper.findAll('.archive-row-detailed')).toHaveLength(2)
  })

  it('searches name and host through one box', async () => {
    const wrapper = selector()
    await pickGroupMode(wrapper, 'Flat')

    await wrapper.find('.archive-controls input').setValue('db-01')
    await nextTick()

    expect(wrapper.findAll('.archive-name').map((n) => n.text())).toEqual([DB.name])
  })

  it('says so when the search matches nothing', async () => {
    const wrapper = selector()
    await wrapper.find('.archive-controls input').setValue('nothing-like-this')
    await nextTick()

    expect(wrapper.find('.archive-no-match').text()).toContain('No archives match')
  })

  it('reorders through the sort select', async () => {
    const wrapper = selector()
    await pickGroupMode(wrapper, 'Flat')

    await wrapper.find('.archive-sort-select').setValue('size-desc')
    await nextTick()

    expect(wrapper.findAll('.archive-name').map((n) => n.text())).toEqual([DB.name, WEB.name])
  })

  it('reports the picked archive through the model', async () => {
    const wrapper = selector()
    await wrapper.findAll('.archive-row-select')[0].trigger('click')

    expect(wrapper.emitted('update:selected')?.[0]).toEqual([DB])
  })

  it('forwards a row delete request with the archive it belongs to', async () => {
    const wrapper = selector({ canDelete: true })
    await wrapper.findAll('.archive-row-delete')[0].trigger('click')

    expect(wrapper.emitted('delete')?.[0]).toEqual([DB])
  })

  it('marks only the archives whose delete is already queued', () => {
    const wrapper = selector({ canDelete: true, deletingNames: [DB.name] })
    const pending = wrapper.findAll('.archive-row-pending')
    expect(pending).toHaveLength(1)
    expect(wrapper.findAll('.archive-row')[0].classes()).toContain('archive-row--deleting')
  })

  it('shows placeholder rows while loading, and nothing else', () => {
    const wrapper = selector({ loading: true })
    expect(wrapper.find('.archive-loading').exists()).toBe(true)
    expect(wrapper.find('.archive-controls').exists()).toBe(false)
    expect(wrapper.find('.archive-row').exists()).toBe(false)
  })

  it('shows a failed load instead of an empty list', () => {
    const wrapper = selector({ archives: [], error: 'Connection refused' })
    expect(wrapper.find('.error-banner').text()).toBe('Connection refused')
    expect(wrapper.find('.empty-state').exists()).toBe(false)
  })

  it('uses the caller wording for an empty repository', () => {
    const wrapper = selector({
      archives: [],
      emptyTitle: 'No archives',
      emptyDescription: 'No backup archives found for this schedule.',
    })
    expect(wrapper.find('.empty-state').text()).toContain('this schedule')
  })

  it('drops the controls and rows for a caller filtered to one archive', () => {
    const wrapper = selector({ hideControls: true })
    expect(wrapper.find('.archive-controls').exists()).toBe(false)
    expect(wrapper.find('.archive-row').exists()).toBe(false)
    // The count still reports what the repository holds.
    expect(wrapper.find('.archive-count').text()).toBe('2')
  })

  it('renders the actions slot in the panel header', () => {
    const wrapper = mount(ArchiveSelector, {
      global: { stubs: { RouterLink: RouterLinkStub } },
      props: { archives: [WEB], selected: null },
      slots: { actions: '<button class="stub-action">Diff</button>' },
    })
    expect(wrapper.find('.panel-header .stub-action').exists()).toBe(true)
  })
})
