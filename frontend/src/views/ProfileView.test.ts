// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { flushPromises, type DOMWrapper } from '@vue/test-utils'
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
  const totpTab = wrapper.findAll('.tab').filter((t) => t.text() === 'Two-factor auth')
  await totpTab[0].trigger('click')
  await wrapper.vm.$nextTick()
}

async function clickSetupButton(wrapper: ReturnType<typeof renderWithPlugins>) {
  const setupBtn = wrapper.findAll('button').find((b) => b.text().includes('Set up two-factor'))
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
  const disableBtn = wrapper.findAll('button').find((b) => b.text().includes('Disable two-factor'))
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

    expect(wrapper.find('.page-description').text()).toContain('admin')
  })

  it('renders the Change Password, API Tokens, and Appearance tabs', () => {
    const wrapper = renderWithPlugins(ProfileView, {
      storeState: { auth: { user: { ...baseUser } } },
    })

    const tabs = wrapper.findAll('.tab')
    expect(tabs.some((t) => t.text() === 'Change password')).toBe(true)
    expect(tabs.some((t) => t.text() === 'API tokens')).toBe(true)
    expect(tabs.some((t) => t.text() === 'Two-factor auth')).toBe(true)
    expect(tabs.some((t) => t.text() === 'Sessions')).toBe(true)
    expect(tabs.some((t) => t.text() === 'Appearance')).toBe(true)
  })

  it('shows TOTP disabled status and setup button', async () => {
    const wrapper = await renderInTotpTab(false)

    expect(wrapper.text()).toContain('not enabled')
    expect(wrapper.text()).toContain('Set up two-factor')
  })

  it('shows TOTP enabled status when totp is active', async () => {
    const wrapper = await renderInTotpTab(true)

    expect(wrapper.text()).toContain('enabled')
    expect(wrapper.text()).toContain('Disable two-factor')
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
    expect(wrapper.text()).toContain('Set up two-factor')
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
      const tokensTab = wrapper.findAll('.tab').filter((t) => t.text() === 'API tokens')
      await tokensTab[0].trigger('click')
      await wrapper.vm.$nextTick()
    }

    function findInModal(
      wrapper: ReturnType<typeof renderWithPlugins>,
      text: string,
    ): DOMWrapper<Element> | undefined {
      return wrapper.findAll('.modal-dialog *').find((el) => el.text().trim() === text)
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

      expect(wrapper.find('.modal-title').text()).toBe('Delete token')

      const overlay = wrapper.find('.modal-backdrop')
      expect(overlay.exists()).toBe(true)
      await overlay.trigger('mousedown')
      await flushPromises()
      expect(wrapper.find('.modal-backdrop').exists()).toBe(false)
    })

    it('closes delete token modal via close button', async () => {
      vi.mocked(apiClient.get).mockResolvedValue({ data: { tokens: [mockToken] } })

      const wrapper = renderWithPlugins(ProfileView, {
        storeState: { auth: { user: { ...baseUser } } },
      })

      await openDeleteTokenModal(wrapper)

      const closeBtn = wrapper.find('button.modal-close')
      expect(closeBtn.exists()).toBe(true)
      await closeBtn.trigger('click')
      await flushPromises()
      expect(wrapper.find('.modal-backdrop').exists()).toBe(false)
    })

    it('closes delete token modal via Cancel button', async () => {
      vi.mocked(apiClient.get).mockResolvedValue({ data: { tokens: [mockToken] } })

      const wrapper = renderWithPlugins(ProfileView, {
        storeState: { auth: { user: { ...baseUser } } },
      })

      await openDeleteTokenModal(wrapper)

      const cancelBtn = findInModal(wrapper, 'Cancel')
      expect(cancelBtn).toBeDefined()
      await cancelBtn!.trigger('click')
      await flushPromises()
      expect(wrapper.find('.modal-backdrop').exists()).toBe(false)
    })

    it('deletes a token from the modal', async () => {
      vi.mocked(apiClient.get).mockResolvedValue({ data: { tokens: [mockToken] } })
      vi.mocked(apiClient.delete).mockResolvedValue({ data: {} })

      const wrapper = renderWithPlugins(ProfileView, {
        storeState: { auth: { user: { ...baseUser } } },
      })

      await openDeleteTokenModal(wrapper)

      const confirmDeleteBtn = findInModal(wrapper, 'Delete')
      expect(confirmDeleteBtn).toBeDefined()
      await confirmDeleteBtn!.trigger('click')
      await flushPromises()

      expect(vi.mocked(apiClient.delete)).toHaveBeenCalledWith('/tokens/1')
    })

    it('shows an error message when deleting a token fails', async () => {
      vi.mocked(apiClient.get).mockResolvedValue({ data: { tokens: [mockToken] } })
      vi.mocked(apiClient.delete).mockRejectedValue(new Error('network error'))

      const wrapper = renderWithPlugins(ProfileView, {
        storeState: { auth: { user: { ...baseUser } } },
      })

      await openDeleteTokenModal(wrapper)

      const confirmDeleteBtn = findInModal(wrapper, 'Delete')
      expect(confirmDeleteBtn).toBeDefined()
      await confirmDeleteBtn!.trigger('click')
      await flushPromises()

      expect(wrapper.find('.modal-dialog').text()).toContain(
        'Failed to delete token: network error',
      )
    })

    /** Opens the dialog, submits a name, and returns with the reveal shown. */
    async function createTokenSuccessfully(): Promise<ReturnType<typeof renderWithPlugins>> {
      mockGetTokens()
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { token: mockToken, plaintext: 'tok_secret' },
      } as never)

      const wrapper = renderWithPlugins(ProfileView)
      await openCreateTokenModal(wrapper)
      await wrapper.find('.modal-dialog input').setValue('ci-token')
      await findInModal(wrapper, 'Create')!.trigger('click')
      await flushPromises()
      return wrapper
    }

    async function openCreateTokenModal(wrapper: ReturnType<typeof renderWithPlugins>) {
      await flushPromises()
      await clickApiTokensTab(wrapper)
      const createBtn = wrapper.findAll('button').find((b) => b.text().trim() === 'New token')
      expect(createBtn).toBeDefined()
      await createBtn!.trigger('click')
      await flushPromises()
    }

    // An empty token list is a place to start, not a dead end: the state
    // says what a token is for and offers the same action as the header
    // button, rather than a bare centred sentence.
    it('offers token creation from the empty state', async () => {
      mockGetTokens()
      const wrapper = renderWithPlugins(ProfileView)
      await flushPromises()
      await clickApiTokensTab(wrapper)

      const empty = wrapper.find('.empty-state')
      expect(empty.find('.empty-title').text()).toBe('No API tokens yet')
      expect(empty.find('.empty-description').text()).toContain(
        'authenticate without your password',
      )

      await empty.find('.empty-action').trigger('click')
      await flushPromises()

      expect(wrapper.find('.modal-dialog').exists()).toBe(true)
      expect(wrapper.find('.modal-title').text()).toContain('token')
    })

    it('creates a token from the name typed in the dialog', async () => {
      mockGetTokens()
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { token: mockToken, plaintext: 'tok_secret' },
      } as never)

      const wrapper = renderWithPlugins(ProfileView)
      await openCreateTokenModal(wrapper)

      await wrapper.find('.modal-dialog input').setValue('ci-token')
      const submit = findInModal(wrapper, 'Create')
      await submit!.trigger('click')
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith('/tokens', { name: 'ci-token' })
    })

    // Enter is a real submit path here, not decoration - the dialog is a
    // single field and typing then pressing Enter is the obvious gesture.
    it('creates the token on Enter as well as the button', async () => {
      mockGetTokens()
      vi.mocked(apiClient.post).mockResolvedValue({
        data: { token: mockToken, plaintext: 'tok_secret' },
      } as never)

      const wrapper = renderWithPlugins(ProfileView)
      await openCreateTokenModal(wrapper)

      const input = wrapper.find('.modal-dialog input')
      await input.setValue('ci-token')
      await input.trigger('keydown.enter')
      await flushPromises()

      expect(apiClient.post).toHaveBeenCalledWith('/tokens', { name: 'ci-token' })
    })

    // The plaintext is shown once and never again, so the dialog swaps to a
    // reveal step rather than closing on success.
    it('reveals the plaintext once, with a copy action', async () => {
      const wrapper = await createTokenSuccessfully()

      expect(wrapper.find('.token-text').text()).toBe('tok_secret')
      // Addressed by container: this file's useClipboard mock is always
      // truthy, so the button reads "Copied" rather than "Copy" here.
      const copy = wrapper.find('.token-box button')
      expect(copy.exists()).toBe(true)
      await copy.trigger('click')
      await flushPromises()
    })

    it('clears the revealed token when the dialog is closed', async () => {
      const wrapper = await createTokenSuccessfully()

      const close = wrapper.findAll('.modal-dialog button').find((b) => /Done|Close/.test(b.text()))
      await close!.trigger('click')
      await flushPromises()

      expect(wrapper.find('.token-text').exists()).toBe(false)
    })

    it('reports a create failure without revealing a token', async () => {
      mockGetTokens()
      vi.mocked(apiClient.post).mockRejectedValue(new Error('name taken'))

      const wrapper = renderWithPlugins(ProfileView)
      await openCreateTokenModal(wrapper)
      await wrapper.find('.modal-dialog input').setValue('ci-token')
      await findInModal(wrapper, 'Create')!.trigger('click')
      await flushPromises()

      expect(wrapper.find('.token-text').exists()).toBe(false)
      expect(wrapper.find('.modal-dialog').text()).toContain('Failed to create token')
    })
  })
})
