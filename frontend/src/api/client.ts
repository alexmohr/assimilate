// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import axios from 'axios'

let redirectingToLogin = false

export const apiClient = axios.create({
  baseURL: '/api',
  headers: { 'Content-Type': 'application/json' },
})

apiClient.interceptors.response.use(
  (response) => response,
  async (error) => {
    const url = error.config?.url ?? ''
    const skipUrls = ['/auth/login', '/auth/me']
    const shouldSkip = skipUrls.some((s) => url.endsWith(s))

    if (error.response?.status === 401 && !shouldSkip) {
      if (!redirectingToLogin) {
        redirectingToLogin = true
        const { router } = await import('../router')
        await router.push({ name: 'login' })
        redirectingToLogin = false
      }
    }
    return Promise.reject(error)
  },
)
