// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { renderWithPlugins } from '../test-utils'
import ProfileView from './ProfileView.vue'
import { apiClient } from '../api/client'

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

const baseUser = {
  id: 1,
  username: 'admin',
  role: 'admin',
  must_change_password: false,
  totp_enabled: false,
  created_at: '2026-01-01T00:00:00Z',
  last_login_at: null,
}

const mockSessions = (sessions: Array<Record<string, unknown>>) => {
  vi.mocked(apiClient.get).mockImplementation((url: string) => {
    if (url === '/auth/sessions') {
      return Promise.resolve({ data: { sessions } })
    }
    return Promise.resolve({ data: { tokens: [] } })
  })
}

function mockGetTokens(): void {
  mockSessions([])
}

async function clickTotpTab(wrapper: ReturnType<typeof renderWithPlugins>) {
  const totpTab = wrapper.findAll('.tab').filter((t) => t.text() === 'Two-Factor Auth')
  await totpTab[0].trigger('click')
  await wrapper.vm.$nextTick()
}

async function clickSetupButton(wrapper: ReturnType<typeof renderWithPlugins>) {
  const setupBtn = wrapper.findAll('button').find((b) => b.text().includes('Set Up Two-Factor'))
  await setupBtn!.trigger('click')
  await flushPromises()
}

async function clickSessionsTab(wrapper: ReturnType<typeof renderWithPlugins>) {
  const sessionsTab = wrapper.findAll('.tab').filter((t) => t.text() === 'Sessions')
  await sessionsTab[0].trigger('click')
  await flushPromises()
}

function totpSetupResponse(recoveryCodes: string[]): {
  secret: string
  qr_uri: string
  recovery_codes: string[]
} {
  return {
    secret: 'ABC234',
    qr_uri: 'data:image/png;base64,QUE=',
    recovery_codes: recoveryCodes,
  }
}

async function renderInTotpTab(
  totpEnabled: boolean,
): Promise<ReturnType<typeof renderWithPlugins>> {
  const wrapper = renderWithPlugins(ProfileView, {
    storeState: { auth: { user: { ...baseUser, totp_enabled: totpEnabled } } },
  })
  await clickTotpTab(wrapper)
  return wrapper
}

async function startTotpSetup(
  recoveryCodes: string[],
): Promise<ReturnType<typeof renderWithPlugins>> {
  mockGetTokens()
  vi.mocked(apiClient.post).mockResolvedValueOnce({ data: totpSetupResponse(recoveryCodes) })
  const wrapper = await renderInTotpTab(false)
  await clickSetupButton(wrapper)
  return wrapper
}

async function clickDisableTotp(
  wrapper: ReturnType<typeof renderWithPlugins>,
  password: string,
): Promise<void> {
  await wrapper.find('input[placeholder="Current password"]').setValue(password)
  const disableBtn = wrapper.findAll('button').find((b) => b.text().includes('Disable Two-Factor'))
  await disableBtn!.trigger('click')
  await flushPromises()
}

const defaultSessionList = [
  {
    id: 'cur',
    user_id: 1,
    created_at: '2026-07-01T00:00:00Z',
    expires_at: '2026-07-08T00:00:00Z',
    last_seen_at: '2026-07-07T00:00:00Z',
    remember_me: true,
    current: true,
  },
  {
    id: 'other',
    user_id: 1,
    created_at: '2026-07-02T00:00:00Z',
    expires_at: '2026-07-03T00:00:00Z',
    last_seen_at: '2026-07-02T00:00:00Z',
    remember_me: false,
    current: false,
  },
]

describe('ProfileView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders the profile page title', () => {
    const wrapper = renderWithPlugins(ProfileView, {
      storeState: { auth: { user: { ...baseUser } } },
    })

    expect(wrapper.find('.page-title').text()).toBe('Profile')
  })

  it('displays the authenticated username', () => {
    const wrapper = renderWithPlugins(ProfileView, {
      storeState: { auth: { user: { ...baseUser } } },
    })

    expect(wrapper.find('.page-subtitle').text()).toContain('admin')
  })

  it('renders the Change Password, API Tokens, and Appearance tabs', () => {
    const wrapper = renderWithPlugins(ProfileView, {
      storeState: { auth: { user: { ...baseUser } } },
    })

    const tabs = wrapper.findAll('.tab')
    expect(tabs.some((t) => t.text() === 'Change Password')).toBe(true)
    expect(tabs.some((t) => t.text() === 'API Tokens')).toBe(true)
    expect(tabs.some((t) => t.text() === 'Two-Factor Auth')).toBe(true)
    expect(tabs.some((t) => t.text() === 'Sessions')).toBe(true)
    expect(tabs.some((t) => t.text() === 'Appearance')).toBe(true)
  })

  it('shows TOTP disabled status and setup button', async () => {
    const wrapper = await renderInTotpTab(false)

    expect(wrapper.text()).toContain('not enabled')
    expect(wrapper.text()).toContain('Set Up Two-Factor')
  })

  it('shows TOTP enabled status when totp is active', async () => {
    const wrapper = await renderInTotpTab(true)

    expect(wrapper.text()).toContain('enabled')
    expect(wrapper.text()).toContain('Disable Two-Factor')
  })

  it('sessions tab loads and displays when clicked', async () => {
    mockSessions([
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
    ])

    const wrapper = renderWithPlugins(ProfileView, {
      storeState: { auth: { user: { ...baseUser } } },
    })

    await clickSessionsTab(wrapper)

    expect(wrapper.text()).toContain('Current')
    expect(wrapper.text()).toContain('Active')
    expect(wrapper.text()).toContain('Remember Me')
  })

  it('starts TOTP setup and shows the QR/secret and verify input', async () => {
    const wrapper = await startTotpSetup(['aa-11', 'bb-22'])

    expect(wrapper.find('img.qr-code').exists()).toBe(true)
    expect(wrapper.text()).toContain('ABC234')
    const verifyInput = wrapper.find('input[placeholder="000000"]')
    expect(verifyInput.exists()).toBe(true)
  })

  it('shows an error when TOTP setup fails', async () => {
    mockGetTokens()
    vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('setup failed'))

    const wrapper = renderWithPlugins(ProfileView, {
      storeState: { auth: { user: { ...baseUser, totp_enabled: false } } },
    })

    await clickTotpTab(wrapper)
    await clickSetupButton(wrapper)

    expect(wrapper.text()).toContain('setup failed')
  })

  it('verifies a TOTP code and displays recovery codes for saving', async () => {
    const wrapper = await startTotpSetup(['aa-11', 'bb-22'])

    await wrapper.find('input[placeholder="000000"]').setValue('123456')
    const verifyBtn = wrapper.findAll('button').find((b) => b.text().includes('Verify & Enable'))
    await verifyBtn!.trigger('click')
    await flushPromises()

    expect(wrapper.findAll('.recovery-code').length).toBe(2)
    const savedBtn = wrapper.findAll('button').find((b) => b.text().includes('I have saved'))
    expect(savedBtn).toBeDefined()
    await savedBtn!.trigger('click')
    await wrapper.vm.$nextTick()
    expect(wrapper.findAll('.recovery-code').length).toBe(0)
  })

  it('cancels TOTP setup', async () => {
    const wrapper = await startTotpSetup(['aa-11'])

    const cancelBtn = wrapper.findAll('button').find((b) => b.text() === 'Cancel')
    await cancelBtn!.trigger('click')
    await wrapper.vm.$nextTick()

    expect(wrapper.find('input[placeholder="000000"]').exists()).toBe(false)
  })

  it('shows an error when TOTP verification fails', async () => {
    const wrapper = await startTotpSetup(['aa-11', 'bb-22'])
    vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('wrong code'))

    await wrapper.find('input[placeholder="000000"]').setValue('000000')
    const verifyBtn = wrapper.findAll('button').find((b) => b.text().includes('Verify & Enable'))
    await verifyBtn!.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain('wrong code')
  })

  it('disables TOTP with the current password', async () => {
    mockGetTokens()
    vi.mocked(apiClient.post).mockResolvedValueOnce({ data: {} })

    const wrapper = await renderInTotpTab(true)
    await clickDisableTotp(wrapper, 'passw0rd')

    expect(vi.mocked(apiClient.post)).toHaveBeenCalledWith('/auth/totp/disable', {
      password: 'passw0rd',
    })
    expect(wrapper.text()).toContain('Set Up Two-Factor')
  })

  it('shows an error when disabling TOTP fails', async () => {
    mockGetTokens()
    vi.mocked(apiClient.post).mockRejectedValueOnce(new Error('disable failed'))

    const wrapper = await renderInTotpTab(true)
    await clickDisableTotp(wrapper, 'passw0rd')

    expect(wrapper.text()).toContain('disable failed')
  })

  it('shows an error when loading sessions fails', async () => {
    vi.mocked(apiClient.get).mockImplementation((url: string) => {
      if (url === '/auth/sessions') {
        return Promise.reject(new Error('load sessions failed'))
      }
      return Promise.resolve({ data: { tokens: [] } })
    })

    const wrapper = renderWithPlugins(ProfileView, {
      storeState: { auth: { user: { ...baseUser } } },
    })

    await clickSessionsTab(wrapper)

    expect(wrapper.text()).toContain('load sessions failed')
  })

  it('revokes a non-current session after confirmation', async () => {
    mockSessions(defaultSessionList)

    const wrapper = renderWithPlugins(ProfileView, {
      storeState: { auth: { user: { ...baseUser } } },
    })

    await clickSessionsTab(wrapper)

    const revokeBtn = wrapper.find('button[title="Revoke session"]')
    expect(revokeBtn.exists()).toBe(true)
    await revokeBtn.trigger('click')
    await wrapper.vm.$nextTick()

    const confirmBtn = Array.from(document.body.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('Revoke'),
    )
    expect(confirmBtn).toBeDefined()
    confirmBtn!.click()
    await flushPromises()

    expect(vi.mocked(apiClient.delete)).toHaveBeenCalledWith('/auth/sessions/other')
  })

  it('shows an error when revoking a session fails', async () => {
    mockSessions(defaultSessionList)
    vi.mocked(apiClient.delete).mockRejectedValueOnce(new Error('session already gone'))

    const wrapper = renderWithPlugins(ProfileView, {
      storeState: { auth: { user: { ...baseUser } } },
    })

    await clickSessionsTab(wrapper)

    const revokeBtn = wrapper.find('button[title="Revoke session"]')
    await revokeBtn.trigger('click')
    await wrapper.vm.$nextTick()

    const confirmBtn = Array.from(document.body.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('Revoke'),
    )
    confirmBtn!.click()
    await flushPromises()

    expect(document.body.textContent).toContain('session already gone')
  })

  it('cancels session revocation without deleting', async () => {
    mockSessions(defaultSessionList)

    const wrapper = renderWithPlugins(ProfileView, {
      storeState: { auth: { user: { ...baseUser } } },
    })

    await clickSessionsTab(wrapper)

    const revokeBtn = wrapper.find('button[title="Revoke session"]')
    await revokeBtn.trigger('click')
    await wrapper.vm.$nextTick()

    const cancelBtn = Array.from(document.body.querySelectorAll('button')).find(
      (b) => b.textContent?.trim() === 'Cancel',
    )
    expect(cancelBtn).toBeDefined()
    cancelBtn!.click()
    await flushPromises()

    expect(vi.mocked(apiClient.delete)).not.toHaveBeenCalled()
  })

  describe('API tokens', () => {
    const mockToken = {
      id: 1,
      name: 'ci-token',
      created_at: '2026-07-01T00:00:00Z',
      last_used_at: null,
    }

    async function clickApiTokensTab(wrapper: ReturnType<typeof renderWithPlugins>) {
      const tokensTab = wrapper.findAll('.tab').filter((t) => t.text() === 'API Tokens')
      await tokensTab[0].trigger('click')
      await wrapper.vm.$nextTick()
    }

    function findInBody(text: string): HTMLElement | undefined {
      return Array.from(document.body.querySelectorAll('*')).find(
        (el) => el.textContent?.trim() === text,
      ) as HTMLElement | undefined
    }

    async function openDeleteTokenModal(wrapper: ReturnType<typeof renderWithPlugins>) {
      await flushPromises()
      await clickApiTokensTab(wrapper)

      const deleteBtn = wrapper.find('button[title="Delete"]')
      expect(deleteBtn.exists()).toBe(true)
      await deleteBtn.trigger('click')
      await wrapper.vm.$nextTick()
    }

    it('shows delete token modal and closes it via overlay click', async () => {
      vi.mocked(apiClient.get).mockResolvedValue({ data: { tokens: [mockToken] } })

      const wrapper = renderWithPlugins(ProfileView, {
        storeState: { auth: { user: { ...baseUser } } },
      })

      await openDeleteTokenModal(wrapper)

      const dialogTitle = findInBody('Delete Token')
      expect(dialogTitle).toBeDefined()

      const overlay = document.querySelector('.overlay') as HTMLElement
      expect(overlay).not.toBeNull()
      overlay.click()
      await flushPromises()
    })

    it('closes delete token modal via close button', async () => {
      vi.mocked(apiClient.get).mockResolvedValue({ data: { tokens: [mockToken] } })

      const wrapper = renderWithPlugins(ProfileView, {
        storeState: { auth: { user: { ...baseUser } } },
      })

      await openDeleteTokenModal(wrapper)

      const closeBtns = document.body.querySelectorAll('button.close-btn')
      expect(closeBtns.length).toBeGreaterThanOrEqual(1)
      closeBtns[0].click()
      await flushPromises()
    })

    it('closes delete token modal via Cancel button', async () => {
      vi.mocked(apiClient.get).mockResolvedValue({ data: { tokens: [mockToken] } })

      const wrapper = renderWithPlugins(ProfileView, {
        storeState: { auth: { user: { ...baseUser } } },
      })

      await openDeleteTokenModal(wrapper)

      const cancelBtn = findInBody('Cancel')
      expect(cancelBtn).toBeDefined()
      cancelBtn!.click()
      await flushPromises()
    })

    it('deletes a token from the modal', async () => {
      vi.mocked(apiClient.get).mockResolvedValue({ data: { tokens: [mockToken] } })
      vi.mocked(apiClient.delete).mockResolvedValue({ data: {} })

      const wrapper = renderWithPlugins(ProfileView, {
        storeState: { auth: { user: { ...baseUser } } },
      })

      await openDeleteTokenModal(wrapper)

      const confirmDeleteBtn = findInBody('Delete')
      expect(confirmDeleteBtn).toBeDefined()
      confirmDeleteBtn!.click()
      await flushPromises()

      expect(vi.mocked(apiClient.delete)).toHaveBeenCalledWith('/tokens/1')
    })
  })
})
