// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { apiClient } from '../api/client'
import { logger } from '../utils/logger'

export interface AuthUser {
  id: number
  username: string
  role: string
  must_change_password: boolean
  created_at: string
  last_login_at: string | null
  totp_enabled?: boolean
}

// Refresh the session when this much time remains before expiry.
const REFRESH_THRESHOLD_MS = 24 * 60 * 60 * 1000

// `role` is an open RBAC role name (custom roles are supported), but "admin"
// is the one built-in role the UI special-cases.
const ADMIN_ROLE_NAME = 'admin'

export const useAuthStore = defineStore('auth', () => {
  const user = ref<AuthUser | null>(null)
  const isAdmin = computed(() => user.value?.role === ADMIN_ROLE_NAME)
  const loading = ref(false)
  const sessionExpiresAt = ref<string | null>(null)
  const rememberMe = ref(false)
  let refreshTimer: ReturnType<typeof setTimeout> | null = null

  // TOTP login flow state
  const totpRequired = ref(false)
  const tempToken = ref<string | null>(null)

  function scheduleRefresh(expiresAt: string): void {
    if (refreshTimer !== null) {
      clearTimeout(refreshTimer)
      refreshTimer = null
    }
    const msUntilExpiry = new Date(expiresAt).getTime() - Date.now()
    const delay = msUntilExpiry - REFRESH_THRESHOLD_MS
    if (delay > 0) {
      refreshTimer = setTimeout(() => void doRefresh(), delay)
    } else if (msUntilExpiry > 0) {
      void doRefresh()
    }
  }

  async function doRefresh(): Promise<void> {
    try {
      const res = await apiClient.post<{ session_expires_at: string }>('/auth/refresh')
      sessionExpiresAt.value = res.data.session_expires_at
      scheduleRefresh(res.data.session_expires_at)
    } catch (e: unknown) {
      logger.debug('session refresh failed', e)
    }
  }

  async function fetchMe(): Promise<void> {
    try {
      const res = await apiClient.get<
        AuthUser & {
          session_expires_at: string | null
          remember_me: boolean
          totp_enabled: boolean
        }
      >('/auth/me')
      user.value = res.data
      if (res.data.remember_me && res.data.session_expires_at) {
        rememberMe.value = true
        sessionExpiresAt.value = res.data.session_expires_at
        scheduleRefresh(res.data.session_expires_at)
      }
    } catch (e: unknown) {
      logger.debug('fetchMe: not authenticated', e)
      user.value = null
    }
  }

  async function login(username: string, password: string, remember = false): Promise<void> {
    const res = await apiClient.post<{
      user: AuthUser
      session_expires_at: string
      remember_me: boolean
      totp_required: boolean
      temp_token: string | null
    }>('/auth/login', {
      username,
      password,
      remember_me: remember,
    })

    if (res.data.totp_required) {
      totpRequired.value = true
      tempToken.value = res.data.temp_token
      return
    }

    user.value = res.data.user
    rememberMe.value = res.data.remember_me
    sessionExpiresAt.value = res.data.session_expires_at
    if (remember) {
      scheduleRefresh(res.data.session_expires_at)
    }

    totpRequired.value = false
    tempToken.value = null
  }

  async function verifyTotp(code: string, recovery = false): Promise<void> {
    if (recovery) {
      // Recovery code endpoint creates the real session directly
      const res = await apiClient.post<{
        user: AuthUser
        session_expires_at: string
        remember_me: boolean
      }>('/auth/totp/recovery', {
        code,
        temp_token: tempToken.value,
      })

      user.value = res.data.user
      rememberMe.value = res.data.remember_me
      sessionExpiresAt.value = res.data.session_expires_at
      if (res.data.remember_me) {
        scheduleRefresh(res.data.session_expires_at)
      }

      totpRequired.value = false
      tempToken.value = null
      return
    }

    // Complete the login with TOTP verification
    const res = await apiClient.post<{
      user: AuthUser
      session_expires_at: string
      remember_me: boolean
    }>('/auth/totp/verify-login', {
      code,
      temp_token: tempToken.value,
    })

    user.value = res.data.user
    rememberMe.value = res.data.remember_me
    sessionExpiresAt.value = res.data.session_expires_at
    if (res.data.remember_me) {
      scheduleRefresh(res.data.session_expires_at)
    }

    totpRequired.value = false
    tempToken.value = null
  }

  async function changePassword(newPassword: string): Promise<void> {
    await apiClient.post('/auth/change-password', { new_password: newPassword })
    if (user.value) {
      user.value.must_change_password = false
    }
  }

  async function logout(): Promise<void> {
    try {
      await apiClient.post('/auth/logout')
    } finally {
      if (refreshTimer !== null) {
        clearTimeout(refreshTimer)
        refreshTimer = null
      }
      user.value = null
      sessionExpiresAt.value = null
      rememberMe.value = false
      totpRequired.value = false
      tempToken.value = null
      window.location.assign('/login')
    }
  }

  function resetTotpState(): void {
    totpRequired.value = false
    tempToken.value = null
  }

  return {
    user,
    loading,
    isAdmin,
    fetchMe,
    login,
    verifyTotp,
    changePassword,
    logout,
    totpRequired,
    tempToken,
    resetTotpState,
  }
})
