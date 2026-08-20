// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { cancelThenConfirmDelete, renderWithPlugins } from '../test-utils'
import TokensView from './TokensView.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
  },
}))

const mockCopyToClipboard = vi.fn()
vi.mock('../composables/useClipboard', () => ({
  useClipboard: () => ({ copied: { value: false }, copy: mockCopyToClipboard }),
}))

vi.mock('../utils/format', () => ({
  formatDate: (v: string | null | undefined, fallback = '') => v ?? fallback,
}))

import { apiClient } from '../api/client'

interface ApiToken {
  id: number
  user_id: number
  name: string
  created_at: string
  last_used_at: string | null
}

const mockTokens: ApiToken[] = [
  {
    id: 1,
    user_id: 1,
    name: 'CI pipeline',
    created_at: '2026-01-01T00:00:00Z',
    last_used_at: null,
  },
  {
    id: 2,
    user_id: 1,
    name: 'deploy-bot',
    created_at: '2026-01-02T00:00:00Z',
    last_used_at: '2026-05-01T00:00:00Z',
  },
]

const mockApiGet = apiClient.get as ReturnType<typeof vi.fn>

beforeEach(() => {
  vi.clearAllMocks()
  mockApiGet.mockResolvedValue({ data: { tokens: mockTokens } })
})

describe('TokensView', () => {
  it('renders token names after loading', async () => {
    const wrapper = renderWithPlugins(TokensView)

    await flushPromises()

    expect(wrapper.text()).toContain('CI pipeline')
    expect(wrapper.text()).toContain('deploy-bot')
  })

  // F-47, the same gap UsersView had: before this the view rendered an empty
  // table on a failed load, which reads as "no tokens exist" rather than "we
  // could not ask".
  it('explains a failed load instead of showing an empty table', async () => {
    mockApiGet.mockRejectedValue(new Error('tokens unavailable'))

    const wrapper = renderWithPlugins(TokensView)
    await flushPromises()

    expect(wrapper.find('.error-banner').text()).toContain('tokens unavailable')
    expect(wrapper.find('table').exists()).toBe(false)
  })

  it('renders New button for creating tokens', async () => {
    const wrapper = renderWithPlugins(TokensView)

    await flushPromises()

    const buttons = wrapper.findAll('button')
    const newButton = buttons.find((b) => b.text().includes('New'))
    expect(newButton).toBeDefined()
  })

  it('opens create token modal on New button click', async () => {
    const wrapper = renderWithPlugins(TokensView)

    await flushPromises()

    const buttons = wrapper.findAll('button')
    const newButton = buttons.find((b) => b.text().includes('New'))
    await newButton!.trigger('click')

    expect(wrapper.text()).toContain('New API token')
  })

  it('renders token table with correct headers', async () => {
    const wrapper = renderWithPlugins(TokensView)

    await flushPromises()

    const headers = wrapper.findAll('th')
    const headerText = headers.map((h) => h.text())
    expect(headerText).toContain('Name')
    expect(headerText).toContain('Created')
    expect(headerText).toContain('Last used')
  })

  it('does not display token secret in the list (masked)', async () => {
    const wrapper = renderWithPlugins(TokensView)

    await flushPromises()

    expect(wrapper.find('.token-text').exists()).toBe(false)
  })

  it('shows empty state when no tokens exist', async () => {
    mockApiGet.mockResolvedValue({ data: { tokens: [] } })

    const wrapper = renderWithPlugins(TokensView)

    await flushPromises()

    expect(wrapper.text()).toContain('No API tokens')
  })

  it('opens the create modal from the empty state action', async () => {
    mockApiGet.mockResolvedValue({ data: { tokens: [] } })

    const wrapper = renderWithPlugins(TokensView)
    await flushPromises()

    await wrapper.find('.empty-action').trigger('click')

    expect(wrapper.text()).toContain('New API token')
  })

  it('closes the create modal via the close button and clicking the overlay', async () => {
    const wrapper = renderWithPlugins(TokensView)
    await flushPromises()

    await wrapper
      .findAll('button')
      .find((b) => b.text().includes('New'))!
      .trigger('click')
    expect(wrapper.text()).toContain('New API token')

    await wrapper.find('button.modal-close').trigger('click')
    expect(wrapper.find('.modal-backdrop').exists()).toBe(false)

    await wrapper
      .findAll('button')
      .find((b) => b.text().includes('New'))!
      .trigger('click')
    expect(wrapper.text()).toContain('New API token')

    await wrapper.find('.modal-backdrop').trigger('mousedown')
    expect(wrapper.find('.modal-backdrop').exists()).toBe(false)
  })

  it('creates a token and shows the plaintext once', async () => {
    const mockApiPost = apiClient.post as ReturnType<typeof vi.fn>
    mockApiPost.mockResolvedValue({
      data: { token: mockTokens[0], plaintext: 'secret-token-value' },
    })
    const wrapper = renderWithPlugins(TokensView)
    await flushPromises()

    const buttons = wrapper.findAll('button')
    await buttons.find((b) => b.text().includes('New'))!.trigger('click')

    await wrapper.find('#token-name').setValue('deploy-key')
    await wrapper.find('form').trigger('submit')
    await flushPromises()

    expect(mockApiPost).toHaveBeenCalledWith('/tokens', { name: 'deploy-key' })
    expect(wrapper.text()).toContain('secret-token-value')
    expect(wrapper.text()).toContain('Token created')

    await wrapper.find('.token-box button').trigger('click')
    expect(mockCopyToClipboard).toHaveBeenCalledWith('secret-token-value')

    await wrapper
      .findAll('button')
      .find((b) => b.text() === 'Done')!
      .trigger('click')
    await flushPromises()
    expect(wrapper.find('.modal-backdrop').exists()).toBe(false)
  })

  it('cancels the delete confirm dialog, then deletes a token once confirmed', async () => {
    const mockApiDelete = apiClient.delete as ReturnType<typeof vi.fn>
    mockApiDelete.mockResolvedValue({ data: {} })
    const wrapper = renderWithPlugins(TokensView)
    await flushPromises()
    expect(wrapper.text()).toContain('CI pipeline')

    await cancelThenConfirmDelete(wrapper, mockApiDelete, '/tokens/1')
  })

  it('shows an error when deleting a token fails', async () => {
    const mockApiDelete = apiClient.delete as ReturnType<typeof vi.fn>
    mockApiDelete.mockRejectedValue(new Error('network error'))
    const wrapper = renderWithPlugins(TokensView)
    await flushPromises()

    const deleteButton = wrapper.findAll('button.btn-danger-text')[0]
    await deleteButton!.trigger('click')
    await wrapper.find('button.btn-danger').trigger('click')
    await flushPromises()

    expect(wrapper.find('.form-error').exists()).toBe(true)
    expect(wrapper.find('.modal-backdrop').exists()).toBe(true)
  })
})
