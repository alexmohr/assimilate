// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import axios, { isCancel } from 'axios'
import type { AxiosRequestConfig } from 'axios'

/** Extra attempts for a GET that failed before any response arrived. */
const MAX_TRANSPORT_RETRIES = 2
const TRANSPORT_RETRY_DELAY_MS = 300

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
  config: AxiosRequestConfig | undefined,
  hasResponse: boolean,
  cancelled: boolean,
  attempts: number,
): boolean {
  if (hasResponse || cancelled || !config) return false
  if (methodKind(config.method) !== 'get') return false
  return attempts < MAX_TRANSPORT_RETRIES
}

/**
 * The retries run inside this one interceptor call, through plain `axios`
 * (which has no interceptors), not `apiClient.request`: re-entering the chain
 * would run the later interceptors (backend-status tracking) once per attempt,
 * so a single failed read would count as up to three.
 */
apiClient.interceptors.response.use(
  (response) => response,
  async (error) => {
    let last = error
    let attempts = 0
    while (
      shouldRetryTransport(last.config, last.response !== undefined, isCancel(last), attempts)
    ) {
      attempts++
      await new Promise<void>((resolve) => setTimeout(resolve, TRANSPORT_RETRY_DELAY_MS))
      try {
        return await axios.request(last.config)
      } catch (e: unknown) {
        last = e
      }
    }
    if (last.response?.status === 401 && !last.config?.url?.includes('/auth/')) {
      window.location.assign('/login')
    }
    return Promise.reject(last)
  },
)
