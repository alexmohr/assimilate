// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import ProfileView from './ProfileView.vue'

vi.mock('../utils/format', () => ({
  formatDate: vi.fn((v: string | null | undefined, fallback = '') => v ?? fallback),
}))

vi.mock('../api/client', () => ({
  apiClient: {
    get: vi.fn().mockResolvedValue({ data: { tokens: [] } }),
    post: vi.fn().mockResolvedValue({ data: {} }),
    delete: vi.fn().mockResolvedValue({ data: {} }),
  },
}))

vi.mock('../composables/useTheme', () => ({
  useTheme: vi.fn(() => ({
    theme: 'auto',
    setTheme: vi.fn(),
    loadFromBackend: vi.fn(),
  })),
}))

vi.mock('../composables/useEscapeKey', () => ({
  useEscapeKey: vi.fn(),
}))

vi.mock('../composables/useClipboard', () => ({
  useClipboard: vi.fn(() => ({
    copied: { value: false },
    copy: vi.fn(),
  })),
}))

describe('ProfileView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders the profile page title', () => {
    const wrapper = renderWithPlugins(ProfileView, {
      storeState: {
        auth: {
          user: {
            id: 1,
            username: 'testuser',
            role: 'admin',
            must_change_password: false,
            created_at: '2026-01-01T00:00:00Z',
            last_login_at: null,
          },
        },
      },
    })

    expect(wrapper.find('.page-title').text()).toBe('Profile')
  })

  it('displays the authenticated username', () => {
    const wrapper = renderWithPlugins(ProfileView, {
      storeState: {
        auth: {
          user: {
            id: 1,
            username: 'testuser',
            role: 'admin',
            must_change_password: false,
            created_at: '2026-01-01T00:00:00Z',
            last_login_at: null,
          },
        },
      },
    })

    expect(wrapper.find('.page-subtitle').text()).toContain('testuser')
  })

  it('renders the Change Password, API Tokens, and Appearance tabs', () => {
    const wrapper = renderWithPlugins(ProfileView, {
      storeState: {
        auth: {
          user: {
            id: 1,
            username: 'admin',
            role: 'admin',
            must_change_password: false,
            created_at: '2026-01-01T00:00:00Z',
            last_login_at: null,
          },
        },
      },
    })

    const tabs = wrapper.findAll('.tab')
    expect(tabs.some((t) => t.text() === 'Change Password')).toBe(true)
    expect(tabs.some((t) => t.text() === 'API Tokens')).toBe(true)
    expect(tabs.some((t) => t.text() === 'Appearance')).toBe(true)
  })
})
