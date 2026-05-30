// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import ExcludesView from './ExcludesView.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    delete: vi.fn(),
  },
}))

import { apiClient } from '../api/client'

const mockGet = vi.mocked(apiClient.get)

const MOCK_EXCLUDES = [
  { id: 1, pattern: 'node_modules', sort_order: 0 },
  { id: 2, pattern: '__pycache__', sort_order: 1 },
]

describe('ExcludesView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders page title', async () => {
    mockGet.mockResolvedValue({ data: MOCK_EXCLUDES })
    const wrapper = renderWithPlugins(ExcludesView)
    await flushPromises()
    expect(wrapper.text()).toContain('Global Excludes')
  })

  it('populates textarea with loaded patterns', async () => {
    mockGet.mockResolvedValue({ data: MOCK_EXCLUDES })
    const wrapper = renderWithPlugins(ExcludesView)
    await flushPromises()
    const textarea = wrapper.find<HTMLTextAreaElement>('textarea')
    expect(textarea.element.value).toContain('node_modules')
    expect(textarea.element.value).toContain('__pycache__')
  })

  it('renders Save button', async () => {
    mockGet.mockResolvedValue({ data: MOCK_EXCLUDES })
    const wrapper = renderWithPlugins(ExcludesView)
    await flushPromises()
    const saveBtn = wrapper.findAll('button').find((b) => b.text() === 'Save')
    expect(saveBtn).toBeDefined()
  })

  it('renders Pattern Reference toggle button', async () => {
    mockGet.mockResolvedValue({ data: MOCK_EXCLUDES })
    const wrapper = renderWithPlugins(ExcludesView)
    await flushPromises()
    expect(wrapper.text()).toContain('Pattern Reference')
  })

  it('shows pattern reference panel when button is clicked', async () => {
    mockGet.mockResolvedValue({ data: MOCK_EXCLUDES })
    const wrapper = renderWithPlugins(ExcludesView)
    await flushPromises()
    const refBtn = wrapper.findAll('button').find((b) => b.text().includes('Pattern Reference'))
    await refBtn!.trigger('click')
    expect(wrapper.text()).toContain('Borg Pattern Syntax')
  })

  it('shows error when API fails', async () => {
    mockGet.mockRejectedValue(new Error('Network error'))
    const wrapper = renderWithPlugins(ExcludesView)
    await flushPromises()
    expect(wrapper.find('.state-error').exists()).toBe(true)
  })

  it('renders empty textarea when no excludes returned', async () => {
    mockGet.mockResolvedValue({ data: [] })
    const wrapper = renderWithPlugins(ExcludesView)
    await flushPromises()
    const textarea = wrapper.find<HTMLTextAreaElement>('textarea')
    expect(textarea.element.value).toBe('')
  })

  it('calls GET /excludes on mount', async () => {
    mockGet.mockResolvedValue({ data: MOCK_EXCLUDES })
    renderWithPlugins(ExcludesView)
    await flushPromises()
    expect(mockGet).toHaveBeenCalledWith('/excludes')
  })
})
