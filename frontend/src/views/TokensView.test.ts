// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import TokensView from './TokensView.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
  },
}))

vi.mock('../composables/useClipboard', () => ({
  useClipboard: () => ({ copied: { value: false }, copy: vi.fn() }),
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

    expect(wrapper.text()).toContain('Create API Token')
  })

  it('renders token table with correct headers', async () => {
    const wrapper = renderWithPlugins(TokensView)

    await flushPromises()

    const headers = wrapper.findAll('th')
    const headerText = headers.map((h) => h.text())
    expect(headerText).toContain('Name')
    expect(headerText).toContain('Created')
    expect(headerText).toContain('Last Used')
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
})
