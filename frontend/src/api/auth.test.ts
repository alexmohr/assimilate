// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { apiClient } from './client'

vi.mock('./client')

import {
  changePassword,
  disableTotp,
  getCurrentUser,
  getPreferences,
  listSessions,
  login,
  logout,
  refreshSession,
  revokeSession,
  setupTotp,
  updatePreferences,
  verifyTotp,
  verifyTotpLogin,
} from './auth'

describe('auth api', () => {
  beforeEach(() => {
    vi.mocked(apiClient.get).mockReset()
    vi.mocked(apiClient.post).mockReset()
    vi.mocked(apiClient.put).mockReset()
    vi.mocked(apiClient.delete).mockReset()
  })

  it('refreshes the session', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({
      data: { session_expires_at: '2026-08-27T00:00:00Z' },
    })

    await expect(refreshSession()).resolves.toEqual({
      session_expires_at: '2026-08-27T00:00:00Z',
    })
    expect(apiClient.post).toHaveBeenCalledWith('/auth/refresh')
  })

  it('gets the current user', async () => {
    const data = {
      id: 1,
      username: 'admin',
      role: 'admin',
      must_change_password: false,
      created_at: '2026-01-01T00:00:00Z',
      last_login_at: null,
      can_upgrade_agent: true,
      session_expires_at: '2026-08-27T00:00:00Z',
      remember_me: true,
      totp_enabled: false,
    }
    vi.mocked(apiClient.get).mockResolvedValue({ data })

    await expect(getCurrentUser()).resolves.toEqual(data)
    expect(apiClient.get).toHaveBeenCalledWith('/auth/me')
  })

  it('logs in with username and password', async () => {
    const result = {
      user: { id: 1, username: 'alice' },
      session_expires_at: '2026-08-22T00:00:00Z',
      remember_me: true,
      totp_required: false,
      temp_token: null,
    }
    vi.mocked(apiClient.post).mockResolvedValue({ data: result })

    await expect(login('alice', 'secret', true)).resolves.toEqual(result)

    expect(apiClient.post).toHaveBeenCalledWith('/auth/login', {
      username: 'alice',
      password: 'secret',
      remember_me: true,
    })
  })

  it('verifies a TOTP login code against verify-login', async () => {
    const result = {
      user: { id: 1, username: 'alice' },
      session_expires_at: '2026-08-22T00:00:00Z',
      remember_me: false,
    }
    vi.mocked(apiClient.post).mockResolvedValue({ data: result })

    await expect(verifyTotpLogin('123456', 'temp-token', false)).resolves.toEqual(result)

    expect(apiClient.post).toHaveBeenCalledWith('/auth/totp/verify-login', {
      code: '123456',
      temp_token: 'temp-token',
    })
  })

  it('verifies a TOTP login recovery code against recovery', async () => {
    const result = {
      user: { id: 1, username: 'alice' },
      session_expires_at: '2026-08-22T00:00:00Z',
      remember_me: false,
    }
    vi.mocked(apiClient.post).mockResolvedValue({ data: result })

    await expect(verifyTotpLogin('recovery-code', 'temp-token', true)).resolves.toEqual(result)

    expect(apiClient.post).toHaveBeenCalledWith('/auth/totp/recovery', {
      code: 'recovery-code',
      temp_token: 'temp-token',
    })
  })

  it('changes the password', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await changePassword('new-password')

    expect(apiClient.post).toHaveBeenCalledWith('/auth/change-password', {
      new_password: 'new-password',
    })
  })

  it('logs out', async () => {
    vi.mocked(apiClient.post).mockResolvedValue({})

    await logout()

    expect(apiClient.post).toHaveBeenCalledWith('/auth/logout')
  })

  it('sets up TOTP', async () => {
    const data = { secret: 'ABC234', qr_uri: 'data:image/png;base64,QUE=', recovery_codes: ['a'] }
    vi.mocked(apiClient.post).mockResolvedValue({ data })

    await expect(setupTotp()).resolves.toEqual(data)
    expect(apiClient.post).toHaveBeenCalledWith('/auth/totp/setup')
  })

  it('verifies a TOTP setup code', async () => {
    const data = { success: true, backup_codes_remaining: 9 }
    vi.mocked(apiClient.post).mockResolvedValue({ data })

    await expect(verifyTotp('654321')).resolves.toEqual(data)
    expect(apiClient.post).toHaveBeenCalledWith('/auth/totp/verify', { code: '654321' })
  })

  it('disables TOTP', async () => {
    const data = { success: true, backup_codes_remaining: null }
    vi.mocked(apiClient.post).mockResolvedValue({ data })

    await expect(disableTotp('my-password')).resolves.toEqual(data)
    expect(apiClient.post).toHaveBeenCalledWith('/auth/totp/disable', { password: 'my-password' })
  })

  it('lists sessions', async () => {
    const sessions = [
      {
        id: 'sess-1',
        user_id: 1,
        created_at: '2026-07-01T00:00:00Z',
        expires_at: '2026-07-08T00:00:00Z',
        last_seen_at: '2026-07-07T00:00:00Z',
        remember_me: true,
        current: true,
      },
    ]
    vi.mocked(apiClient.get).mockResolvedValue({ data: { sessions } })

    await expect(listSessions()).resolves.toEqual(sessions)
    expect(apiClient.get).toHaveBeenCalledWith('/auth/sessions')
  })

  it('revokes a session', async () => {
    vi.mocked(apiClient.delete).mockResolvedValue({})

    await revokeSession('sess-1')

    expect(apiClient.delete).toHaveBeenCalledWith('/auth/sessions/sess-1')
  })

  it('gets preferences', async () => {
    vi.mocked(apiClient.get).mockResolvedValue({ data: { theme: 'dark' } })

    await expect(getPreferences()).resolves.toEqual({ theme: 'dark' })
    expect(apiClient.get).toHaveBeenCalledWith('/auth/preferences')
  })

  it('updates preferences', async () => {
    vi.mocked(apiClient.put).mockResolvedValue({})

    await updatePreferences('dark')

    expect(apiClient.put).toHaveBeenCalledWith('/auth/preferences', { theme: 'dark' })
  })
})
