// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import AgentHostnameAliases from './AgentHostnameAliases.vue'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), post: vi.fn(), delete: vi.fn() },
}))

const PATTERNS = [
  { id: 1, agent_id: 3, pattern: 'web-*', created_at: '2026-01-01T00:00:00Z' },
  { id: 2, agent_id: 3, pattern: 'web-??', created_at: '2026-01-02T00:00:00Z' },
]

async function mount(props: Record<string, unknown> = {}) {
  const wrapper = renderWithPlugins(AgentHostnameAliases, {
    props: { hostname: 'web-01', canEdit: true, ...props },
  })
  await flushPromises()
  return wrapper
}

describe('AgentHostnameAliases', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get)
      .mockReset()
      .mockResolvedValue({ data: PATTERNS } as never)
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.delete)
      .mockReset()
      .mockResolvedValue({} as never)
  })

  it('loads the host patterns on mount', async () => {
    const wrapper = await mount()
    expect(apiClient.get).toHaveBeenCalledWith('/agents/web-01/hostname-patterns')
    expect(wrapper.text()).toContain('web-*')
    expect(wrapper.text()).toContain('web-??')
  })

  it('reloads when the view switches to another host', async () => {
    const wrapper = await mount()
    await wrapper.setProps({ hostname: 'db-01' })
    await flushPromises()
    expect(apiClient.get).toHaveBeenLastCalledWith('/agents/db-01/hostname-patterns')
  })

  it('says so when no patterns are configured', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: [] } as never)
    const wrapper = await mount()
    expect(wrapper.text()).toContain('No alias patterns configured.')
  })

  it('adds a pattern and appends it to the list without a refetch', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { id: 9, agent_id: 3, pattern: 'app-*', created_at: '2026-02-01T00:00:00Z' },
    } as never)
    const wrapper = await mount()

    await wrapper.find('input').setValue('app-*')
    await wrapper.find('.pattern-add-row button').trigger('click')
    await flushPromises()

    expect(apiClient.post).toHaveBeenCalledWith('/agents/web-01/hostname-patterns', {
      pattern: 'app-*',
    })
    expect(wrapper.text()).toContain('app-*')
    expect(apiClient.get).toHaveBeenCalledOnce()
    // The field is cleared so the next pattern starts fresh.
    expect(wrapper.find('input').element.value).toBe('')
  })

  it('refuses to add a blank pattern', async () => {
    const wrapper = await mount()
    await wrapper.find('input').setValue('   ')
    expect(wrapper.find('.pattern-add-row button').attributes('disabled')).toBeDefined()
  })

  it('surfaces the server error when adding fails', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('pattern already exists'))
    const wrapper = await mount()

    await wrapper.find('input').setValue('web-*')
    await wrapper.find('.pattern-add-row button').trigger('click')
    await flushPromises()

    expect(wrapper.find('.form-error').text()).toContain('pattern already exists')
  })

  it('deletes a pattern and drops it from the list', async () => {
    const wrapper = await mount()
    await wrapper.find('.pattern-delete').trigger('click')
    await flushPromises()

    expect(apiClient.delete).toHaveBeenCalledWith('/agents/web-01/hostname-patterns/1')
    expect(wrapper.text()).not.toContain('web-*')
  })

  it('hides the add row and the delete buttons for an imported host', async () => {
    const wrapper = await mount({ canEdit: false })
    expect(wrapper.find('.pattern-add-row').exists()).toBe(false)
    expect(wrapper.find('.pattern-delete').exists()).toBe(false)
  })

  it('exposes a reload for the rename flow that adds the old name as an alias', async () => {
    const wrapper = await mount()
    await (wrapper.vm as unknown as { reload: (h?: string) => Promise<void> }).reload('renamed-01')
    expect(apiClient.get).toHaveBeenLastCalledWith('/agents/renamed-01/hostname-patterns')
  })

  // The alias list is a side panel on the agent page. A failed load is logged
  // rather than surfaced, so a transient error does not put an error banner
  // on a page whose main content loaded fine.
  it('stays quiet when the alias list fails to load', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('unreachable'))
    const wrapper = await mount()

    expect(wrapper.find('.form-error').exists()).toBe(false)
    expect(wrapper.findAll('.pattern-row')).toHaveLength(0)
  })

  it('reports a failure to add an alias, and keeps the typed pattern', async () => {
    vi.mocked(apiClient.post).mockRejectedValue(new Error('pattern already exists'))
    const wrapper = await mount()

    const input = wrapper.find('.pattern-add-row input')
    await input.setValue('web-*')
    await wrapper.find('.pattern-add-row button').trigger('click')
    await flushPromises()

    expect(wrapper.find('.form-error').exists()).toBe(true)
    expect((input.element as HTMLInputElement).value).toBe('web-*')
  })

  it('reports a failure to delete an alias and keeps the row', async () => {
    vi.mocked(apiClient.delete).mockRejectedValue(new Error('in use'))
    const wrapper = await mount()
    const before = wrapper.findAll('.pattern-row').length

    await wrapper.find('.pattern-delete').trigger('click')
    await flushPromises()

    expect(wrapper.find('.form-error').exists()).toBe(true)
    expect(wrapper.findAll('.pattern-row')).toHaveLength(before)
  })
})
