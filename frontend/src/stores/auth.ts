// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { defineStore } from 'pinia'
import { ref } from 'vue'
import { apiClient } from '../api/client'
import { logger } from '../utils/logger'

export interface AuthUser {
  id: number
  username: string
  role: 'admin' | 'user'
  must_change_password: boolean
  created_at: string
  last_login_at: string | null
}

export const useAuthStore = defineStore('auth', () => {
  const user = ref<AuthUser | null>(null)
  const loading = ref(false)

  async function fetchMe(): Promise<void> {
    try {
      const res = await apiClient.get<AuthUser>('/auth/me')
      user.value = res.data
    } catch (e: unknown) {
      logger.debug('fetchMe: not authenticated', e)
      user.value = null
    }
  }

  async function login(username: string, password: string, rememberMe = false): Promise<void> {
    const res = await apiClient.post<{ user: AuthUser }>('/auth/login', {
      username,
      password,
      remember_me: rememberMe,
    })
    user.value = res.data.user
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
      user.value = null
      window.location.assign('/login')
    }
  }

  return { user, loading, fetchMe, login, changePassword, logout }
})
