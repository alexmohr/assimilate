// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useAuthStore } from './auth'

vi.mock('../api/client', () => ({
  apiClient: {
    post: vi.fn(),
    get: vi.fn(),
  },
}))

const locationAssign = vi.fn()
vi.stubGlobal('window', {
  location: { assign: locationAssign },
})

const defaultUser = {
  id: 1,
  username: 'user',
  role: 'admin',
  must_change_password: false,
  created_at: '2026-01-01T00:00:00Z',
  last_login_at: null,
}

describe('auth store - TOTP flow', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
  })

  afterEach(() => {
    locationAssign.mockClear()
  })

  it('sets totpRequired and tempToken on login when totp_required is true', async () => {
    const { apiClient } = await import('../api/client')
    vi.mocked(apiClient.post).mockResolvedValueOnce({
      data: {
        user: null,
        session_expires_at: '2026-07-21T12:00:00Z',
        remember_me: false,
        totp_required: true,
        temp_token: 'temp-abc-123',
      },
    })

    const store = useAuthStore()
    await store.login('user', 'pass')

    expect(store.totpRequired).toBe(true)
    expect(store.tempToken).toBe('temp-abc-123')
    expect(store.user).toBeNull()
  })

  it('clears totp state on normal login without totp', async () => {
    const { apiClient } = await import('../api/client')
    vi.mocked(apiClient.post).mockResolvedValueOnce({
      data: {
        user: defaultUser,
        session_expires_at: '2026-07-22T12:00:00Z',
        remember_me: false,
        totp_required: false,
        temp_token: null,
      },
    })

    const store = useAuthStore()
    store.totpRequired = true
    store.tempToken = 'old-temp'
    await store.login('user', 'pass')

    expect(store.totpRequired).toBe(false)
    expect(store.tempToken).toBeNull()
  })

  it('verifyTotp with recovery code completes login', async () => {
    const { apiClient } = await import('../api/client')
    vi.mocked(apiClient.post).mockResolvedValueOnce({
      data: {
        user: defaultUser,
        session_expires_at: '2026-07-22T12:00:00Z',
        remember_me: true,
      },
    })

    const store = useAuthStore()
    store.tempToken = 'temp-recovery'
    store.totpRequired = true
    await store.verifyTotp('recovery-code-123', true)

    expect(apiClient.post).toHaveBeenCalledWith('/auth/totp/recovery', {
      code: 'recovery-code-123',
      temp_token: 'temp-recovery',
    })
    expect(store.user).toEqual(defaultUser)
    expect(store.totpRequired).toBe(false)
    expect(store.tempToken).toBeNull()
  })

  it('verifyTotp with TOTP code completes login', async () => {
    const { apiClient } = await import('../api/client')
    vi.mocked(apiClient.post).mockResolvedValueOnce({
      data: {
        user: defaultUser,
        session_expires_at: '2026-07-22T12:00:00Z',
        remember_me: true,
      },
    })

    const store = useAuthStore()
    store.tempToken = 'temp-totp'
    store.totpRequired = true
    await store.verifyTotp('123456', false)

    expect(apiClient.post).toHaveBeenCalledWith('/auth/totp/verify-login', {
      code: '123456',
      temp_token: 'temp-totp',
    })
    expect(store.user).toEqual(defaultUser)
    expect(store.totpRequired).toBe(false)
    expect(store.tempToken).toBeNull()
  })

  it('logout clears totp state', async () => {
    const { apiClient } = await import('../api/client')
    vi.mocked(apiClient.post).mockResolvedValueOnce({ data: {} })

    const store = useAuthStore()
    store.user = defaultUser
    store.totpRequired = true
    store.tempToken = 'some-temp'
    await store.logout()

    expect(store.totpRequired).toBe(false)
    expect(store.tempToken).toBeNull()
    expect(locationAssign).toHaveBeenCalledWith('/login')
  })

  it('resetTotpState clears totp fields', () => {
    const store = useAuthStore()
    store.totpRequired = true
    store.tempToken = 'some-temp'
    store.resetTotpState()

    expect(store.totpRequired).toBe(false)
    expect(store.tempToken).toBeNull()
  })
})
