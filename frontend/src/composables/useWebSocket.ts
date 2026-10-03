// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, onUnmounted } from 'vue'
import type { ServerToUi } from '../types/generated/ServerToUi'
import { logger } from '../utils/logger'

type ServerToUiPayload<T extends ServerToUi['type']> =
  Extract<ServerToUi, { type: T }> extends { payload: infer P } ? P : undefined

export type WsConnectionStatus = 'connected' | 'disconnected' | 'reconnecting'

type OnMessage = {
  <T extends ServerToUi['type']>(type: T, callback: (data: ServerToUiPayload<T>) => void): void
  <T>(type: string, callback: (data: T) => void): void
}

interface UseWebSocketReturn {
  status: ReturnType<typeof ref<WsConnectionStatus>>
  onMessage: OnMessage
  forceReconnect: () => void
}

const MAX_BACKOFF_MS = 30_000

const status = ref<WsConnectionStatus>('disconnected')
const listeners = new Map<string, Set<(data: unknown) => void>>()

let socket: WebSocket | null = null
let backoffMs = 1_000
let reconnectTimer: ReturnType<typeof setTimeout> | null = null

function buildUrl(): string {
  // eslint-disable-next-line local/no-string-literal-control-flow -- window.location.protocol is a DOM API value, not domain state
  const proto = window.location.protocol === 'https:' ? 'wss' : 'ws'
  return `${proto}://${window.location.host}/ws/ui`
}

function connect(): void {
  // Connecting now supersedes any retry that was still waiting to run.
  cancelScheduledReconnect()
  const ws = new WebSocket(buildUrl())
  socket = ws

  // A replaced socket can still deliver events afterwards (a browser fires
  // `close` asynchronously); only the current one may change state.
  const isCurrent = (): boolean => socket === ws

  ws.addEventListener('open', () => {
    if (!isCurrent()) return
    status.value = 'connected'
    backoffMs = 1_000
  })

  ws.addEventListener('message', (event: MessageEvent<string>) => {
    if (!isCurrent()) return
    let parsed: { type: string; payload: unknown }
    try {
      parsed = JSON.parse(event.data) as { type: string; payload: unknown }
    } catch {
      return
    }

    const handlers = listeners.get(parsed.type)
    if (handlers) {
      handlers.forEach((cb) => cb(parsed.payload))
    }
  })

  ws.addEventListener('close', () => {
    if (!isCurrent()) return
    socket = null
    scheduleReconnect()
  })

  ws.addEventListener('error', (ev) => {
    if (!isCurrent()) return
    logger.debug('ws: connection error', ev)
    ws.close()
  })
}

function cancelScheduledReconnect(): void {
  if (reconnectTimer !== null) {
    clearTimeout(reconnectTimer)
    reconnectTimer = null
  }
}

function scheduleReconnect(): void {
  status.value = 'reconnecting'
  cancelScheduledReconnect()
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null
    connect()
    backoffMs = Math.min(backoffMs * 2, MAX_BACKOFF_MS)
  }, backoffMs)
}

function forceReconnect(): void {
  if (socket) {
    socket.close()
  } else {
    connect()
  }
}

connect()

document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'visible' && status.value !== 'connected') {
    backoffMs = 1_000
    if (socket) {
      // Retired before closing, so its close event is ignored instead of
      // scheduling a retry next to the connection opened below.
      const stale = socket
      socket = null
      stale.close()
    }
    status.value = 'reconnecting'
    connect()
  }
})

export function useWebSocket(): UseWebSocketReturn {
  const localHandlers: Array<{ type: string; cb: (data: unknown) => void }> = []

  const onMessage = ((type: string, callback: (data: unknown) => void): void => {
    if (!listeners.has(type)) {
      listeners.set(type, new Set())
    }
    const cb = callback as (data: unknown) => void
    listeners.get(type)!.add(cb)
    localHandlers.push({ type, cb })
  }) as OnMessage

  onUnmounted(() => {
    for (const { type, cb } of localHandlers) {
      listeners.get(type)?.delete(cb)
    }
  })

  return { status, onMessage, forceReconnect }
}
