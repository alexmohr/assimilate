// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import { apiClient } from '../api/client'
import EntityTags from './EntityTags.vue'
import type { TagRow } from '../types/tag'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn(), post: vi.fn(), put: vi.fn() },
}))

const ALL_TAGS: TagRow[] = [
  { id: 1, name: 'production', color: '#ff0000', scope: 'repo' },
  { id: 2, name: 'offsite', color: '#00ff00', scope: 'repo' },
  { id: 3, name: 'archive', color: '#0000ff', scope: 'repo' },
] as unknown as TagRow[]

const ASSIGNED: TagRow[] = [ALL_TAGS[0]]

function mockLoad(all: TagRow[] = ALL_TAGS, own: TagRow[] = ASSIGNED): void {
  vi.mocked(apiClient.get).mockImplementation((url: string) =>
    Promise.resolve({ data: url === '/tags' ? all : own }),
  )
}

function mount(props: Record<string, unknown> = {}) {
  return renderWithPlugins(EntityTags, {
    props: { scope: 'repo', entityPath: '/repos/12', ...props },
  })
}

describe('EntityTags', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put)
      .mockReset()
      .mockResolvedValue({} as never)
    mockLoad()
  })

  it('lists the tags in the requested scope and the ones already assigned', async () => {
    mount()
    await flushPromises()
    expect(apiClient.get).toHaveBeenCalledWith('/tags', {
      params: { scope: 'repo' },
      timeout: undefined,
    })
    expect(apiClient.get).toHaveBeenCalledWith('/repos/12/tags')
  })

  it('reads the scope it was given rather than assuming repositories', async () => {
    mount({ scope: 'host', entityPath: '/agents/web-01' })
    await flushPromises()
    expect(apiClient.get).toHaveBeenCalledWith('/tags', {
      params: { scope: 'host' },
      timeout: undefined,
    })
    expect(apiClient.get).toHaveBeenCalledWith('/agents/web-01/tags')
  })

  it('shows the assigned tags as pills', async () => {
    const wrapper = mount()
    await flushPromises()
    expect(wrapper.findAll('.tag-pill').map((p) => p.text())).toEqual(['production'])
  })

  it('offers only the unassigned tags in the picker', async () => {
    const wrapper = mount()
    await flushPromises()
    const options = wrapper.findAll('option').map((o) => o.text())
    expect(options).toEqual(['Add existing tag...', 'offsite', 'archive'])
  })

  it('says so when nothing is assigned', async () => {
    mockLoad(ALL_TAGS, [])
    const wrapper = mount()
    await flushPromises()
    expect(wrapper.find('.muted').text()).toBe('No tags assigned.')
    expect(wrapper.findAll('.tag-pill')).toHaveLength(0)
  })

  it('hides the picker when every tag is already assigned', async () => {
    mockLoad(ALL_TAGS, ALL_TAGS)
    const wrapper = mount()
    await flushPromises()
    expect(wrapper.find('select').exists()).toBe(false)
  })

  it('reloads when the view switches to another entity', async () => {
    const wrapper = mount()
    await flushPromises()
    vi.mocked(apiClient.get).mockClear()

    await wrapper.setProps({ entityPath: '/repos/99' })
    await flushPromises()

    expect(apiClient.get).toHaveBeenCalledWith('/repos/99/tags')
  })

  // The tag list is a side panel: if the entity's own tags fail to load, the
  // picker should still work rather than the whole panel going blank.
  it('keeps working when the entity tag fetch fails', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) =>
      url === '/tags'
        ? Promise.resolve({ data: ALL_TAGS })
        : Promise.reject(new Error('forbidden')),
    )
    const wrapper = mount()
    await flushPromises()

    expect(wrapper.find('.muted').text()).toBe('No tags assigned.')
    expect(wrapper.findAll('option')).toHaveLength(4)
  })

  it('survives the tag list itself failing', async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new Error('down'))
    const wrapper = mount()
    await flushPromises()
    expect(wrapper.find('.muted').exists()).toBe(true)
  })

  describe('assigning', () => {
    it('adds a picked tag to the existing set rather than replacing it', async () => {
      const wrapper = mount()
      await flushPromises()

      const select = wrapper.find('select')
      ;(select.element as HTMLSelectElement).value = '2'
      await select.trigger('change')
      await flushPromises()

      expect(apiClient.put).toHaveBeenCalledWith('/repos/12/tags', { tag_ids: [1, 2] })
      expect(wrapper.findAll('.tag-pill').map((p) => p.text())).toEqual(['production', 'offsite'])
    })

    it('ignores the placeholder option', async () => {
      const wrapper = mount()
      await flushPromises()

      const select = wrapper.find('select')
      ;(select.element as HTMLSelectElement).value = ''
      await select.trigger('change')
      await flushPromises()

      expect(apiClient.put).not.toHaveBeenCalled()
    })

    it('removes a tag without disturbing the others', async () => {
      mockLoad(ALL_TAGS, [ALL_TAGS[0], ALL_TAGS[1]])
      const wrapper = mount()
      await flushPromises()

      await wrapper.findAll('.tag-remove')[0].trigger('click')
      await flushPromises()

      expect(apiClient.put).toHaveBeenCalledWith('/repos/12/tags', { tag_ids: [2] })
    })

    it('leaves the pills alone when saving fails', async () => {
      vi.mocked(apiClient.put).mockRejectedValueOnce(new Error('conflict'))
      const wrapper = mount()
      await flushPromises()

      await wrapper.find('.tag-remove').trigger('click')
      await flushPromises()

      expect(wrapper.findAll('.tag-pill').map((p) => p.text())).toEqual(['production'])
    })

    it('leaves the pills alone when assigning fails', async () => {
      vi.mocked(apiClient.put).mockRejectedValueOnce(new Error('conflict'))
      const wrapper = mount()
      await flushPromises()

      const select = wrapper.find('select')
      ;(select.element as HTMLSelectElement).value = '2'
      await select.trigger('change')
      await flushPromises()

      expect(wrapper.findAll('.tag-pill').map((p) => p.text())).toEqual(['production'])
    })

    it('labels each remove button with the tag it removes', async () => {
      const wrapper = mount()
      await flushPromises()
      expect(wrapper.find('.tag-remove').attributes('aria-label')).toBe('Remove tag production')
    })
  })

  describe('creating', () => {
    it('cannot be triggered with an empty or blank name', async () => {
      const wrapper = mount()
      await flushPromises()
      const button = wrapper.findAll('button').find((b) => b.text().includes('Create'))!
      expect(button.attributes('disabled')).toBeDefined()

      await wrapper.find('input[aria-label="New tag name"]').setValue('   ')
      expect(button.attributes('disabled')).toBeDefined()
    })

    it('creates the tag in the chosen colour', async () => {
      const created = { id: 9, name: 'nightly', color: '#ff8800', scope: 'repo' } as TagRow
      vi.mocked(apiClient.post).mockResolvedValueOnce({ data: created } as never)

      const wrapper = mount()
      await flushPromises()
      await wrapper.find('input[aria-label="New tag name"]').setValue('nightly')
      await wrapper.find('input[aria-label="New tag colour"]').setValue('#ff8800')
      await wrapper
        .findAll('button')
        .find((b) => b.text().includes('Create'))!
        .trigger('click')
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith('/tags', {
        name: 'nightly',
        color: '#ff8800',
        scope: 'repo',
      })
    })

    it('creates the tag in this scope and assigns it in one go', async () => {
      const created = { id: 9, name: 'nightly', color: '#123456', scope: 'repo' } as TagRow
      vi.mocked(apiClient.post).mockResolvedValueOnce({ data: created } as never)

      const wrapper = mount()
      await flushPromises()
      await wrapper.find('input[aria-label="New tag name"]').setValue('  nightly  ')
      await wrapper
        .findAll('button')
        .find((b) => b.text().includes('Create'))!
        .trigger('click')
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith('/tags', {
        name: 'nightly',
        color: '#6b7280',
        scope: 'repo',
      })
      expect(apiClient.put).toHaveBeenCalledWith('/repos/12/tags', { tag_ids: [1, 9] })
    })

    /** Types a tag name, presses Create, and hands back the name field. */
    async function submitNewTag(wrapper: ReturnType<typeof mount>, name: string) {
      const field = wrapper.find('input[aria-label="New tag name"]')
      await field.setValue(name)
      await wrapper
        .findAll('button')
        .find((b) => b.text().includes('Create'))!
        .trigger('click')
      await flushPromises()
      return field
    }

    // The field is cleared on success and kept on failure: retyping a name
    // the server just rejected would be the wrong thing to ask of the user.
    it('clears the form after a successful create', async () => {
      const created = { id: 9, name: 'nightly', color: '#123456', scope: 'repo' } as TagRow
      vi.mocked(apiClient.post).mockResolvedValueOnce({ data: created } as never)

      const wrapper = mount()
      await flushPromises()
      const name = await submitNewTag(wrapper, 'nightly')

      expect((name.element as HTMLInputElement).value).toBe('')
    })

    it('keeps the typed name when the create fails, so it is not lost', async () => {
      vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('duplicate'))

      const wrapper = mount()
      await flushPromises()
      const name = await submitNewTag(wrapper, 'nightly')

      expect((name.element as HTMLInputElement).value).toBe('nightly')
    })
  })

  it('exposes a reload for the parent view', async () => {
    const wrapper = mount()
    await flushPromises()
    vi.mocked(apiClient.get).mockClear()

    await (wrapper.vm as unknown as { reload: () => Promise<void> }).reload()
    await flushPromises()

    expect(apiClient.get).toHaveBeenCalledWith('/repos/12/tags')
  })
})
