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
    expect(tabs.some((t) => t.text() === 'Two-Factor Auth')).toBe(true)
    expect(tabs.some((t) => t.text() === 'Sessions')).toBe(true)
    expect(tabs.some((t) => t.text() === 'Appearance')).toBe(true)
  })

  it('shows TOTP disabled status and setup button', async () => {
    const wrapper = renderWithPlugins(ProfileView, {
      storeState: {
        auth: {
          user: {
            id: 1,
            username: 'admin',
            role: 'admin',
            must_change_password: false,
            totp_enabled: false,
            created_at: '2026-01-01T00:00:00Z',
            last_login_at: null,
          },
        },
      },
    })

    // Navigate to the TOTP tab
    const totpTab = wrapper.findAll('.tab').filter((t) => t.text() === 'Two-Factor Auth')
    await totpTab[0].trigger('click')

    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('not enabled')
    expect(wrapper.text()).toContain('Set Up Two-Factor')
  })

  it('shows TOTP enabled status when totp is active', async () => {
    const wrapper = renderWithPlugins(ProfileView, {
      storeState: {
        auth: {
          user: {
            id: 1,
            username: 'admin',
            role: 'admin',
            must_change_password: false,
            totp_enabled: true,
            created_at: '2026-01-01T00:00:00Z',
            last_login_at: null,
          },
        },
      },
    })

    const totpTab = wrapper.findAll('.tab').filter((t) => t.text() === 'Two-Factor Auth')
    await totpTab[0].trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('enabled')
    expect(wrapper.text()).toContain('Disable Two-Factor')
  })

  it('sessions tab loads and displays when clicked', async () => {
    const { apiClient } = await import('../api/client')
    vi.mocked(apiClient.get).mockResolvedValue({
      data: {
        sessions: [
          {
            id: 'sess-1',
            user_id: 1,
            created_at: '2026-07-01T00:00:00Z',
            expires_at: '2026-07-08T00:00:00Z',
            last_seen_at: '2026-07-07T00:00:00Z',
            remember_me: true,
            current: true,
          },
          {
            id: 'sess-2',
            user_id: 1,
            created_at: '2026-07-02T00:00:00Z',
            expires_at: '2026-07-03T00:00:00Z',
            last_seen_at: '2026-07-02T12:00:00Z',
            remember_me: false,
            current: false,
          },
        ],
      },
    })

    const wrapper = renderWithPlugins(ProfileView, {
      storeState: {
        auth: {
          user: {
            id: 1,
            username: 'admin',
            role: 'admin',
            must_change_password: false,
            totp_enabled: false,
            created_at: '2026-01-01T00:00:00Z',
            last_login_at: null,
          },
        },
      },
    })

    const sessionsTab = wrapper.findAll('.tab').filter((t) => t.text() === 'Sessions')
    await sessionsTab[0].trigger('click')
    await wrapper.vm.$nextTick()
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('Current')
    expect(wrapper.text()).toContain('Active')
    expect(wrapper.text()).toContain('Remember Me')
  })
})
