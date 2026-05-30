// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { type ComputedRef, type Ref, ref, computed, watch } from 'vue'
import { useWebSocket } from './useWebSocket'
import { apiClient } from '../api/client'

const RETRY_INTERVAL_MS = 30_000
const FAILURE_THRESHOLD = 2

const consecutiveFailures = ref(0)
const retryCountdown = ref(0)
const checking = ref(false)

let countdownTimer: ReturnType<typeof setInterval> | null = null
let retryTimer: ReturnType<typeof setTimeout> | null = null

const { status: wsStatus, forceReconnect } = useWebSocket()

const backendUnreachable = computed(
  () => wsStatus.value !== 'connected' && consecutiveFailures.value >= FAILURE_THRESHOLD,
)

function startCountdown(): void {
  stopCountdown()
  retryCountdown.value = RETRY_INTERVAL_MS / 1_000

  countdownTimer = setInterval(() => {
    retryCountdown.value--
    if (retryCountdown.value <= 0) {
      stopCountdown()
      retryNow()
    }
  }, 1_000)
}

function stopCountdown(): void {
  if (countdownTimer !== null) {
    clearInterval(countdownTimer)
    countdownTimer = null
  }
  if (retryTimer !== null) {
    clearTimeout(retryTimer)
    retryTimer = null
  }
}

async function retryNow(): Promise<void> {
  if (checking.value) return
  checking.value = true
  try {
    await apiClient.get('/health')
    consecutiveFailures.value = 0
    stopCountdown()
    forceReconnect()
  } catch {
    consecutiveFailures.value++
    startCountdown()
  } finally {
    checking.value = false
  }
}

function recordHttpFailure(): void {
  consecutiveFailures.value++
  if (backendUnreachable.value && !countdownTimer) {
    startCountdown()
  }
}

function recordHttpSuccess(): void {
  consecutiveFailures.value = 0
  stopCountdown()
}

watch(wsStatus, (newStatus) => {
  if (newStatus === 'connected') {
    consecutiveFailures.value = 0
    stopCountdown()
  } else if (newStatus === 'reconnecting' && !countdownTimer) {
    retryTimer = setTimeout(() => {
      if (wsStatus.value !== 'connected' && consecutiveFailures.value < FAILURE_THRESHOLD) {
        retryNow()
      }
    }, 3_000)
  }
})

apiClient.interceptors.response.use(
  (response) => {
    recordHttpSuccess()
    return response
  },
  (error) => {
    if (!error.response) {
      recordHttpFailure()
    }
    return Promise.reject(error)
  },
)

export interface UseBackendStatusReturn {
  backendUnreachable: ComputedRef<boolean>
  retryCountdown: Ref<number>
  checking: Ref<boolean>
  retryNow: () => Promise<void>
}

export function useBackendStatus(): UseBackendStatusReturn {
  return {
    backendUnreachable,
    retryCountdown,
    checking,
    retryNow,
  }
}
