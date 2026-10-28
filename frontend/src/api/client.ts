// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import axios, { isCancel } from 'axios'
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

type MethodKind = 'get' | 'other'

function methodKind(method: string | undefined): MethodKind {
  return method?.toLowerCase() === 'get' ? 'get' : 'other'
}

/**
 * A request that dies without any HTTP response (reset keep-alive connection,
 * proxy dropping an idle socket) is retried when it is a GET: reads are
 * idempotent, and a stale pooled connection typically works on the next try.
 * Writes are never replayed, and a request the caller aborted on purpose is
 * not a transport failure, so it is never retried.
 */
function shouldRetryTransport(
  config: RetryableConfig | undefined,
  hasResponse: boolean,
  cancelled: boolean,
): boolean {
  if (hasResponse || cancelled || !config) return false
  if (methodKind(config.method) !== 'get') return false
  return (config.transportRetries ?? 0) < MAX_TRANSPORT_RETRIES
}

apiClient.interceptors.response.use(
  (response) => response,
  async (error) => {
    if (error.response?.status === 401 && !error.config?.url?.includes('/auth/')) {
      window.location.assign('/login')
    }
    const config: RetryableConfig | undefined = error.config
    if (config && shouldRetryTransport(config, error.response !== undefined, isCancel(error))) {
      config.transportRetries = (config.transportRetries ?? 0) + 1
      await new Promise<void>((resolve) => setTimeout(resolve, TRANSPORT_RETRY_DELAY_MS))
      return apiClient.request(config)
    }
    return Promise.reject(error)
  },
)
