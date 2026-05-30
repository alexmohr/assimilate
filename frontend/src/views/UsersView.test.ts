// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import UsersView from './UsersView.vue'

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
  },
}))

vi.mock('../utils/format', () => ({
  formatDate: (v: string | null | undefined, fallback = '') => v ?? fallback,
}))

import { apiClient } from '../api/client'

interface User {
  id: number
  username: string
  role: 'admin' | 'user'
  created_at: string
  last_login_at: string | null
}

const mockUsers: User[] = [
  { id: 1, username: 'admin', role: 'admin', created_at: '2026-01-01T00:00:00Z', last_login_at: null },
  { id: 2, username: 'operator1', role: 'user', created_at: '2026-01-02T00:00:00Z', last_login_at: null },
  { id: 3, username: 'viewer1', role: 'user', created_at: '2026-01-03T00:00:00Z', last_login_at: null },
]

const mockApiGet = apiClient.get as ReturnType<typeof vi.fn>

beforeEach(() => {
  vi.clearAllMocks()
  mockApiGet.mockResolvedValue({ data: mockUsers })
})

describe('UsersView', () => {
  it('renders user list after loading', async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'admin', role: 'admin' } } },
    })

    await flushPromises()

    expect(wrapper.text()).toContain('admin')
    expect(wrapper.text()).toContain('operator1')
    expect(wrapper.text()).toContain('viewer1')
  })

  it('renders a New button for creating users', async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'admin', role: 'admin' } } },
    })

    await flushPromises()

    const buttons = wrapper.findAll('button')
    const newButton = buttons.find((b) => b.text().includes('New'))
    expect(newButton).toBeDefined()
  })

  it('shows "you" badge for the currently authenticated user', async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 1, username: 'admin', role: 'admin' } } },
    })

    await flushPromises()

    expect(wrapper.text()).toContain('you')
  })

  it('does not show "you" badge when no user matches', async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'other', role: 'admin' } } },
    })

    await flushPromises()

    expect(wrapper.text()).not.toContain('you')
  })

  it('shows role badge for each user', async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'other', role: 'admin' } } },
    })

    await flushPromises()

    const badges = wrapper.findAll('.role-badge')
    expect(badges.length).toBe(3)
  })

  it('opens create modal on New button click', async () => {
    const wrapper = renderWithPlugins(UsersView, {
      storeState: { auth: { user: { id: 99, username: 'other', role: 'admin' } } },
    })

    await flushPromises()

    const buttons = wrapper.findAll('button')
    const newButton = buttons.find((b) => b.text().includes('New'))
    await newButton!.trigger('click')
    await flushPromises()

    expect(document.body.textContent).toContain('Add User')
  })
})
