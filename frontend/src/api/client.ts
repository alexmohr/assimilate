// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import axios from 'axios'
import type { InternalAxiosRequestConfig } from 'axios'

/** Extra attempts for a GET that failed before any response arrived. */
const MAX_TRANSPORT_RETRIES = 2
const TRANSPORT_RETRY_DELAY_MS = 300

interface RetryableConfig extends InternalAxiosRequestConfig {
  transportRetries?: number
}

export const apiClient = axios.create({
  baseURL: '/api',
  headers: { 'Content-Type': 'application/json' },
})

function isGetMethod(method: string | undefined): method is 'get' | 'GET' {
  return method?.toLowerCase() === 'get'
}

/**
 * A request that dies without any HTTP response (reset keep-alive connection,
 * proxy dropping an idle socket) is retried when it is a GET: reads are
 * idempotent, and a stale pooled connection typically works on the next try.
 * Writes are never replayed.
 */
function shouldRetryTransport(config: RetryableConfig | undefined, hasResponse: boolean): boolean {
  if (hasResponse || !config) return false
  if (!isGetMethod(config.method)) return false
  return (config.transportRetries ?? 0) < MAX_TRANSPORT_RETRIES
}

apiClient.interceptors.response.use(
  (response) => response,
  async (error) => {
    if (error.response?.status === 401 && !error.config?.url?.includes('/auth/')) {
      window.location.assign('/login')
    }
    const config: RetryableConfig | undefined = error.config
    if (config && shouldRetryTransport(config, error.response !== undefined)) {
      config.transportRetries = (config.transportRetries ?? 0) + 1
      await new Promise<void>((resolve) => setTimeout(resolve, TRANSPORT_RETRY_DELAY_MS))
      return apiClient.request(config)
    }
    return Promise.reject(error)
  },
)
