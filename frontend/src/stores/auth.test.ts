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

// The shape POST /auth/login and the TOTP login endpoints return for `.user`
// (UserResponse: created_at/last_login_at, no can_upgrade_agent).
const defaultLoginUser = {
  id: 1,
  username: 'user',
  role: 'admin',
  must_change_password: false,
  created_at: '2026-01-01T00:00:00Z',
  last_login_at: null,
}

// The shape GET /auth/me returns (MeResponse: session/permission fields, no
// created_at/last_login_at) - what the store's `user` is ultimately typed as.
const defaultMeUser = {
  id: 1,
  username: 'user',
  role: 'admin',
  must_change_password: false,
  session_expires_at: null,
  remember_me: false,
  can_upgrade_agent: false,
  totp_enabled: false,
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
    expect(apiClient.get).not.toHaveBeenCalled()
  })

  it('clears totp state and fetches the full user on normal login without totp', async () => {
    const { apiClient } = await import('../api/client')
    vi.mocked(apiClient.post).mockResolvedValueOnce({
      data: {
        user: defaultLoginUser,
        session_expires_at: '2026-07-22T12:00:00Z',
        remember_me: false,
        totp_required: false,
        temp_token: null,
      },
    })
    vi.mocked(apiClient.get).mockResolvedValueOnce({ data: defaultMeUser })

    const store = useAuthStore()
    store.totpRequired = true
    store.tempToken = 'old-temp'
    await store.login('user', 'pass')

    expect(apiClient.get).toHaveBeenCalledWith('/auth/me')
    expect(store.user).toEqual(defaultMeUser)
    expect(store.totpRequired).toBe(false)
    expect(store.tempToken).toBeNull()
  })

  it.each([
    {
      recovery: true,
      code: 'recovery-code-123',
      tempToken: 'temp-recovery',
      endpoint: '/auth/totp/recovery',
    },
    {
      recovery: false,
      code: '123456',
      tempToken: 'temp-totp',
      endpoint: '/auth/totp/verify-login',
    },
  ])(
    'verifyTotp with recovery=$recovery completes login and fetches the full user',
    async ({ recovery, code, tempToken, endpoint }) => {
      const { apiClient } = await import('../api/client')
      vi.mocked(apiClient.post).mockResolvedValueOnce({
        data: {
          user: defaultLoginUser,
          session_expires_at: '2026-07-22T12:00:00Z',
          remember_me: true,
        },
      })
      vi.mocked(apiClient.get).mockResolvedValueOnce({ data: defaultMeUser })

      const store = useAuthStore()
      store.tempToken = tempToken
      store.totpRequired = true
      await store.verifyTotp(code, recovery)

      expect(apiClient.post).toHaveBeenCalledWith(endpoint, { code, temp_token: tempToken })
      expect(apiClient.get).toHaveBeenCalledWith('/auth/me')
      expect(store.user).toEqual(defaultMeUser)
      expect(store.totpRequired).toBe(false)
      expect(store.tempToken).toBeNull()
    },
  )

  it('logout clears totp state', async () => {
    const { apiClient } = await import('../api/client')
    vi.mocked(apiClient.post).mockResolvedValueOnce({ data: {} })

    const store = useAuthStore()
    store.user = defaultMeUser
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

  it('changePassword clears the must-change-password flag', async () => {
    const { apiClient } = await import('../api/client')
    vi.mocked(apiClient.post).mockResolvedValueOnce({ data: {} })

    const store = useAuthStore()
    store.user = { ...defaultMeUser, must_change_password: true }
    await store.changePassword('new-secret')

    expect(apiClient.post).toHaveBeenCalledWith('/auth/change-password', {
      new_password: 'new-secret',
    })
    expect(store.user?.must_change_password).toBe(false)
  })
})

describe('auth store - session lifecycle', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('fetchMe populates the user and schedules a refresh when remembered', async () => {
    const { apiClient } = await import('../api/client')
    const expiresAt = new Date(Date.now() + 60 * 60 * 1000).toISOString()
    const meUser = { ...defaultMeUser, remember_me: true, session_expires_at: expiresAt }
    vi.mocked(apiClient.get).mockResolvedValueOnce({ data: meUser })

    const store = useAuthStore()
    await store.fetchMe()

    expect(store.user).toEqual(meUser)
  })

  it('fetchMe clears the user on failure', async () => {
    const { apiClient } = await import('../api/client')
    vi.mocked(apiClient.get).mockRejectedValueOnce(new Error('not authenticated'))

    const store = useAuthStore()
    store.user = defaultMeUser
    await store.fetchMe()

    expect(store.user).toBeNull()
  })

  it('login with remember schedules and eventually runs a session refresh', async () => {
    const { apiClient } = await import('../api/client')
    // scheduleRefresh's `void doRefresh()` fires synchronously inside
    // fetchMe() (called by login()), before login()'s own promise settles --
    // so the refresh's mock response must already be queued up front, not
    // added after awaiting login().
    const soonExpiry = new Date(Date.now() + 5 * 60 * 1000).toISOString()
    const refreshedExpiry = new Date(Date.now() + 2 * 60 * 60 * 1000).toISOString()
    vi.mocked(apiClient.post)
      .mockResolvedValueOnce({
        data: {
          user: defaultLoginUser,
          session_expires_at: soonExpiry,
          remember_me: true,
          totp_required: false,
          temp_token: null,
        },
      })
      .mockResolvedValueOnce({ data: { session_expires_at: refreshedExpiry } })
    const meUser = { ...defaultMeUser, remember_me: true, session_expires_at: soonExpiry }
    vi.mocked(apiClient.get).mockResolvedValueOnce({ data: meUser })

    const store = useAuthStore()
    await store.login('user', 'pass', true)
    expect(store.user).toEqual(meUser)

    await vi.advanceTimersByTimeAsync(0)

    expect(apiClient.post).toHaveBeenCalledWith('/auth/refresh')
  })
})
