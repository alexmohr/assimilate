// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import {
  changePassword as apiChangePassword,
  getCurrentUser,
  login as apiLogin,
  logout as apiLogout,
  refreshSession,
  verifyTotpLogin,
} from '../api/auth'
import type { CurrentUserResponse } from '../api/auth'
import { logger } from '../utils/logger'

export type { CurrentUserResponse } from '../api/auth'

// Refresh the session when this much time remains before expiry.
const REFRESH_THRESHOLD_MS = 24 * 60 * 60 * 1000

// `role` is an open RBAC role name (custom roles are supported), but "admin"
// is the one built-in role the UI special-cases.
const ADMIN_ROLE_NAME = 'admin'

export const useAuthStore = defineStore('auth', () => {
  const user = ref<CurrentUserResponse | null>(null)
  const isAdmin = computed(() => user.value?.role === ADMIN_ROLE_NAME)
  const canUpgradeAgent = computed(() => user.value?.can_upgrade_agent ?? false)
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
      const data = await refreshSession()
      sessionExpiresAt.value = data.session_expires_at
      scheduleRefresh(data.session_expires_at)
    } catch (e: unknown) {
      logger.debug('session refresh failed', e)
    }
  }

  async function fetchMe(): Promise<void> {
    try {
      const data = await getCurrentUser()
      user.value = data
      if (data.remember_me && data.session_expires_at) {
        rememberMe.value = true
        sessionExpiresAt.value = data.session_expires_at
        scheduleRefresh(data.session_expires_at)
      }
    } catch (e: unknown) {
      logger.debug('fetchMe: not authenticated', e)
      user.value = null
    }
  }

  async function login(username: string, password: string, remember = false): Promise<void> {
    const data = await apiLogin(username, password, remember)

    if (data.totp_required) {
      totpRequired.value = true
      tempToken.value = data.temp_token
      return
    }

    // The login response's `user` is the leaner UserResponse shape (no
    // can_upgrade_agent) - fetch the full /auth/me record now the session is
    // established, so permission-gated UI is correct without a page reload.
    await fetchMe()

    totpRequired.value = false
    tempToken.value = null
  }

  async function verifyTotp(code: string, recovery = false): Promise<void> {
    await verifyTotpLogin(code, tempToken.value, recovery)

    await fetchMe()

    totpRequired.value = false
    tempToken.value = null
  }

  async function changePassword(newPassword: string): Promise<void> {
    await apiChangePassword(newPassword)
    if (user.value) {
      user.value.must_change_password = false
    }
  }

  async function logout(): Promise<void> {
    try {
      await apiLogout()
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
    canUpgradeAgent,
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
